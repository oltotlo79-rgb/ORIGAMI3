//! 基本技法のマクロ(段折り・中割り折り・かぶせ折り)。
//!
//! 全ての技法は [`fold_through`] の繰り返しと層順序の並べ替えの合成として実装し、
//! 専用のデータ構造を持たない(要件§12のリスク対策: 技法がうまく当てはまらない形でも、
//! 利用者は同じことを手動の折り操作で行える)。
//! [`fold_through`] 自身は汎用の折り操作([`crate::flat_motion`])の包み紙なので、
//! ここも間接的にその上に乗っている。
//!
//! # 手順の記録
//!
//! 手順再生は「そのステップが駆動する全ての折り線」を [`DriverLine`] として必要とする
//! (未指定の折り線は0°に固定される)。そのため技法が生成した折り線は、内部で
//! 何回 [`fold_through`] を呼んでも1つの [`FoldStep`] にまとめて記録する。
//! 技法の中で裏返った部分(段折りの段の中・中割り折りの先端)にもとからあった
//! 折り線は山谷が入れ替わるので、その分もDriverLineとして記録する。
//!
//! # 座標系
//!
//! 入力の座標は全て「畳んだ平面座標」(3D表示のxy)。[`fold_through`] は結果を
//! 「根面(最小面ID)が恒等」の座標系へそろえ直すため、1回折るごとに畳み平面の
//! 座標が全体の等長変換だけずれる。2本目以降の折り線はこのずれを打ち消してから
//! 渡す([`Session::fold`] が返す変換を使う)。
//!
//! # 既知の制限
//!
//! - 層順序は [`fold_through`] の近似(動いた層を山全体の上/下へまとめる)の上に、
//!   先端をもとの層の隣へ置き直して決める([`Session::reorder_tips`])
//! - 重なりの一部だけを選んだフラップなど、紙が裂ける指定・折り上がりの山谷と
//!   重なり順が食い違う指定は、断らずに警告して続ける(「止めずに警告」原則)
//! - 開いてつぶす([`squash`])は既存の折り目を**開く**動きが要るので
//!   [`fold_through`] では組めず、汎用の折り操作([`crate::flat_motion`])を
//!   1回呼ぶマクロとして書いてある(開く動き・回転をそのまま表せる)
//! - 花弁折り([`petal`])も同じく [`crate::flat_motion`] を1回呼ぶマクロ
//!   (左右の羽と中央のくさびに別々の等長変換を与える)

use std::collections::{HashMap, HashSet};

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_geometry::{Isometry2, reflect_across_line};
use ori3_model::{
    CreasePattern, DriverLine, EPS, EdgeId, EdgeKind, FaceId, FoldStep, TechniqueKind, VertexId,
};

use crate::flat_motion::{
    FlatMotionInput, HalfPlane, LayerTurn, MotionPart, MotionTransform, flat_motion,
};
use crate::flat_state::{FlatState, point_in_face, representative_point};
use crate::fold_through::{
    AUX_PROMOTION_WARNING_MARK, FoldDirection, FoldThroughInput, FoldThroughResult, TEAR_MARK,
    angle_of, faces_by_edge, fold_through, push_driver_line, vertex_positions,
};

/// 面の配置の一致を見る許容誤差(等長変換の積み重ねで出る誤差より十分大きく取る)。
const JOIN_EPS: f64 = 1e-6;

/// 「向きが同じ」とみなす角度の許容誤差(rad)。つぶし折りの退化ケースの判定に使う。
const ANGLE_EPS: f64 = 1e-9;

/// 細分化された曲線の隣り合う区間として追跡する最大の折れ角。
/// `ori3-cp::flatfold` が曲線の警告を1件へまとめる判定と同じ45°。
const MAX_CURVE_BEND: f64 = std::f64::consts::FRAC_PI_4;

/// 曲線の各分割点を貫くrulingまたは交差線の共線判定の許容誤差。
const CURVE_RULING_EPS: f64 = 1e-6;

/// 技法の共通入力。座標は全て「畳んだ平面座標」(3D表示のxy)。
#[derive(Clone, Debug)]
pub struct TechniqueInput {
    /// 対象フラップ(畳み平面で選んだ層の面ID)。
    /// 段折りでは空を許し、その場合は折り線の可動側に掛かる全ての層を折る。
    pub flap: Vec<FaceId>,
    /// 折り線(2点。無限直線として扱う)
    pub line: [[f64; 2]; 2],
    /// 技法ごとに意味の変わる基準点(各関数のdocを参照)
    pub reference_point: [f64; 2],
    /// つぶし折り・花弁折りで、動かした紙を重なりのどちら側へ回すか。
    /// `None`/`Some(false)` は手前(いちばん上)、`Some(true)` は向こう(いちばん下)。
    /// 実際の紙ではどちらへも開ける(鶴の基本形は前後に1回ずつ花弁折りする)ので、
    /// 両方を表せるようにしてある。
    /// 段折り・中割り折り・かぶせ折りでは見ない(向きは紙のつながりから決まるため)。
    pub open_to_back: Option<bool>,
    /// ねじり折りの中央多角形(畳み平面の頂点を順に並べる。3点以上)。
    /// 省略すると `line` を1辺として中心のまわりに回した正多角形になる。
    /// 辺ごとに長さの違う多角形は線1本では指せないので、この項目で直接渡す
    /// (半平面はいくつでも並べられるので [`flat_motion`] 側の制限ではない)。
    /// 他の技法では見ない。
    pub polygon: Option<Vec<[f64; 2]>>,
    /// ねじり折りの中心。省略すると選んだ層の重心を使う。他の技法では見ない。
    pub center: Option<[f64; 2]>,
}

/// 段折り(平行な2本の折り線で山・谷を交互に折る)。
///
/// `line` が1本目の折り線、`reference_point` は2本目の折り線の位置を示す点
/// (1本目に平行で `reference_point` を通る直線が2本目)。1本目の可動側は
/// `reference_point` のある側(段になる部分と、その先の紙が動く)。
///
/// 2回の [`fold_through`] の合成で、`reference_point` の先の紙は段の幅の2倍だけずれる。
/// どちらの折りも「動く側を上へ回す」(Up)。段の中は1本目で裏返っているので、
/// 2本目の折り線は展開図では1本目と反対の線種になり、山谷が交互に並ぶ。
/// 重なりは下から「もとの紙・段の中・その先」の順(紙を横から見た Z 字)。
/// 段の中(2本の折り線に挟まれた帯)にもとからあった折り目は、裏返ると同時に
/// 重なり順も入れ替わるため、山谷は変わらない
/// (詳しくは [`Session::fix_reversed_creases`])。
///
/// `flap` が空なら折り線の可動側に掛かる全ての層を折る。
pub fn pleat(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &TechniqueInput,
) -> Result<FoldThroughResult, String> {
    let (l0, l1) = line_points(input.line)?;
    let u = (l1 - l0).normalize();
    let r = DVec2::from(input.reference_point);
    let gap = u.perp_dot(r - l0);
    if gap.abs() <= EPS {
        return Err(
            "段の幅が0です。2本目の折り線の位置を1本目から離して指定してください".to_string(),
        );
    }

    let mut s = Session::new(cp, faces, state)?;
    let flap = s.resolve_flap(&input.flap, "段折り")?;

    // 1本目: reference_point のある側(段とその先)が動く。動かさない側は反対側。
    let normal = DVec2::new(-u.y, u.x);
    let keep1 = r - normal * (gap * 2.0);
    let targets1 = s.movable_targets(input.line, [keep1.x, keep1.y], flap.as_deref());
    let m1 = s.fold(
        input.line,
        [keep1.x, keep1.y],
        targets1.as_deref(),
        FoldDirection::Up,
    )?;

    // 2本目: 1本目で動いた側にあるので、1本目の鏡映を掛けてから新しい平面座標へ写す。
    let moved_map = m1.compose(&Isometry2::reflection(l0, l1));
    let line2 = [moved_map.apply(r), moved_map.apply(r + u)];
    let line2 = [[line2[0].x, line2[0].y], [line2[1].x, line2[1].y]];
    // 2本目の動かさない側=2本の折り線に挟まれた帯(こちらも1本目で動いている)
    let keep2 = moved_map.apply(r - normal * (gap * 0.5));
    // 2本目で折るのは1本目で動いた紙だけ。動かなかった紙(もとの位置に残っている
    // 層)は、2本目の折り線の可動側に掛かっていても折ってはいけない
    let flap2 = s.moved_faces();
    let targets2 = s.movable_targets(line2, [keep2.x, keep2.y], Some(&flap2));
    s.fold(
        line2,
        [keep2.x, keep2.y],
        targets2.as_deref(),
        turn_direction(FoldDirection::Up, m1.mirrored),
    )?;

    // 段の中の折り目を、動いた後の重なり順に合わせ直す
    s.fix_reversed_creases();
    s.finish(TechniqueKind::Pleat, "段折り", cp)
}

/// 中割り折り(フラップの先端を内側へ折り込む)。
///
/// `reference_point` は折り込む先(先端が向かう側)を示す点で、その側は動かない。
/// フラップは2層以上で、折り線が全ての層を横切っている必要がある。
///
/// 折り線が横切る折り目(背)でつながった2つの層は、先端どうしがつながったまま
/// 裏返るので必ず反対向きに回る。この関係で層を2群に塗り分け
/// ([`Session::split_flap_by_connection`])、「下へ回す層」「上へ回す層」の
/// 2回に分けて [`fold_through`] を呼ぶ。層の数が奇数でも、重なりの一部だけを
/// 選んだ場合でも、紙のつながりどおりに折れる。
/// 展開図には同じ線種の折り線が2本(背を挟んでV字に)入る。
/// そのあと先端をもとの層の隣(中割りでは層の**内側**)へ置き直し、
/// 先端の中の背の山谷を新しい重なり順に合わせ直す
/// (中割り折りで背の向きが入れ替わるのはこのため)。
pub fn inside_reverse(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &TechniqueInput,
) -> Result<FoldThroughResult, String> {
    reverse_fold(cp, faces, state, input, true)
}

/// かぶせ折り(フラップの先端を外側へかぶせる)。中割り折りの逆。
///
/// `reference_point` はかぶせる先(先端が向かう側)を示す点で、その側は動かない。
/// いちばん手前(上)の層の先端が上へ回り、つながった層はその反対向きに回るので、
/// 先端はフラップの層の**外側**を包む。層の追い方は [`inside_reverse`] と同じ。
pub fn outside_reverse(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &TechniqueInput,
) -> Result<FoldThroughResult, String> {
    reverse_fold(cp, faces, state, input, false)
}

/// 中割り折り/かぶせ折りの本体(違いは折る向きと、先端を層の内側へ入れるか外側へ出すか)。
fn reverse_fold(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &TechniqueInput,
    inside: bool,
) -> Result<FoldThroughResult, String> {
    let name = if inside {
        "中割り折り"
    } else {
        "かぶせ折り"
    };
    let (l0, l1) = line_points(input.line)?;
    let u = (l1 - l0).normalize();
    let keep = DVec2::from(input.reference_point);
    if u.perp_dot(keep - l0).abs() <= EPS {
        return Err(format!(
            "{name}の向きを示す点が折り線の上にあります。先端を{}側の点を指してください",
            if inside {
                "折り込む先の"
            } else {
                "かぶせる先の"
            }
        ));
    }

    let mut s = Session::new(cp, faces, state)?;
    // フラップの面を層順(下→上)に並べる
    let flap = s.flap_in_layer_order(&input.flap, name)?;
    if flap.len() < 2 {
        return Err(format!(
            "{name}にはフラップが2層以上必要です。重なった層をまとめて選んでください"
        ));
    }
    for &id in &flap {
        if !s.crosses(id, input.line) {
            return Err(format!(
                "このフラップには{name}ができません。折り線がフラップを横切っていないか確認してください"
            ));
        }
    }

    // 紙のつながりから、先端を上へ回す層と下へ回す層に塗り分ける。
    // 折り線が横切る折り目(背)でつながった2層は、先端どうしがつながったまま
    // 裏返るので、必ず反対向きに回る。層の数を機械的に半分に割ると、奇数層や
    // 一部だけを選んだフラップで紙のつながりと食い違ってしまう。
    let up = s.split_flap_by_connection(&flap, input.line, inside, name)?;
    let (up_faces, down_faces): (Vec<FaceId>, Vec<FaceId>) = flap.iter().partition(|id| up[id]);

    // 下へ回す層と上へ回す層をそれぞれ1回の折りにまとめる(どちらかが空になる
    // 指定=つながっていない層だけを選んだ場合は、その回を飛ばす)。
    let keep_point = [keep.x, keep.y];
    let mut line_now = input.line;
    let mut keep_now = keep_point;
    let mut turned = false;
    for (targets, direction) in [
        (&down_faces, FoldDirection::Down),
        (&up_faces, FoldDirection::Up),
    ] {
        let now = s.descendants(targets);
        if now.is_empty() {
            continue;
        }
        let m = s.fold(
            line_now,
            keep_now,
            Some(&now),
            turn_direction(direction, turned),
        )?;
        // 折り線と動かさない側の点は動かない側の幾何なので、平面座標のずれだけを打ち消す
        let a = m.apply(DVec2::from(line_now[0]));
        let b = m.apply(DVec2::from(line_now[1]));
        line_now = [[a.x, a.y], [b.x, b.y]];
        let k = m.apply(DVec2::from(keep_now));
        keep_now = [k.x, k.y];
        turned = turned != m.mirrored;
    }

    // 先端を、それぞれが回った向きの側(上へ回った層は元の層のすぐ上、
    // 下へ回った層はすぐ下)へ置き直し、先端の中の折り目(層と層をつなぐ背)の
    // 山谷を新しい重なり順に合わせ直す。
    // 折るたびに畳み平面ごと裏返ることがあり、そのときは重なり順の上下も
    // 入れ替わっているので、置く側も入れ替える(turn_directionと同じ事情)。
    s.reorder_tips(&up, turned);
    s.fix_reversed_creases();

    let kind = if inside {
        TechniqueKind::InsideReverse
    } else {
        TechniqueKind::OutsideReverse
    };
    s.finish(kind, name, cp)
}

/// 開いてつぶす折り(フラップを開いて平らにつぶす)。
///
/// `line` は**開く中心線**: フラップの背(開く折り目)が乗っている畳み平面の直線。
/// `reference_point` は**つぶす方向を示す点**: 背の自由端が向かう先。
/// 背の一方の端(基準点から遠いほう)が支点になり、背は支点まわりに
/// 「支点→もう一方の端」から「支点→基準点」へ回る(その回転角をαとする)。
///
/// 動きは1回の [`flat_motion`] で表す(要件§12: 専用のデータ構造を持たない):
///
/// - 手前寄りの半分(背でつながった層を2色に塗り分けた、下の層を含むほうの色)は
///   支点を通る**新しい折り線**(背と行き先の角の二等分線M)で折り返す。
///   本体とつながったまま動くのはこちら
/// - 奥の半分は層まるごと回る。鏡映2回(M→行き先の直線)= 角αの回転で、
///   これが「背が開く」動き(背の両側の向きがそろって折り目の角が0°になる)
/// - ただし奥の半分がフラップの外の紙とつながっている(=向こう端が固定されている)
///   ときは、層まるごと回すとそこで必ず紙が裂ける。実際の紙ではこの場合も
///   **両側とも二等分線Mで折り返される**ので、そのように折る。折り返した紙は
///   開いた袋の中(手前の半分と奥の半分の間)へ入り、`open_to_back` は袋の
///   入れ子の向き(手前の紙が上か下か)を選ぶ。予備基本形の4つの袋を開いて
///   つぶす動き(カエルの基本形の下ごしらえ)がこれにあたる
/// - α=0(基準点が背の延長上)は**退化ケース**: 紙は1mmも動かず、重なり順と
///   背の山谷だけが変わる(2回半分に折った正方形を予備基本形の重なりへ組み替える
///   ときの動き)
///
/// つぶした紙を入れる重なりの側は `open_to_back` で選ぶ。既定は手前
/// (いちばん上)、`Some(true)` なら向こう(いちばん下)。実際の紙では
/// どちらへも開けるので、片方に決め打ちしない。
/// 層の数の偶奇やフラップの形は仮定しない。Errにするのは幾何的に決められない
/// 入力(退化した中心線・支点と重なる基準点・見つからない層)だけで、
/// 紙が裂ける・山谷と重なり順が食い違う指定は警告して続ける。
pub fn squash(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &TechniqueInput,
) -> Result<FoldThroughResult, String> {
    let name = "開いてつぶす折り";
    let (l0, l1) = line_points(input.line)?;
    let u = (l1 - l0).normalize();

    let mut flap = flap_in_layer_order(faces, state, &input.flap, name)?;

    let mut warnings: Vec<String> = Vec::new();
    let spine = spine_along(cp, faces, state, &flap, l0, u);
    if spine.curve_ends.is_some() {
        // 曲線ツールのrulingで分割されたfacetは、画面では1枚の花びらとして選ぶ。
        // 選択した1面だけでは曲線の途中で層対応が途切れるため、同じ曲線の全区間で
        // 背を挟む面を論理フラップへ含める。直線の背では従来の明示選択を変えない。
        let curve_faces: HashSet<FaceId> = spine.pairs.iter().flatten().copied().collect();
        flap = state
            .order
            .iter()
            .copied()
            .filter(|id| flap.contains(id) || curve_faces.contains(id))
            .collect();
    }
    let span = match spine.span {
        Some(s) => s,
        None => {
            warnings.push(format!(
                "この{name}では、中心線の上に開ける折り目が見つかりません。中心線がフラップの背に重なっているか確かめてください(指定のまま続行します)"
            ));
            flap_span_along(cp, faces, state, &flap, l0, u).ok_or_else(|| {
                format!("{name}の支点が決められません。中心線を引き直してください")
            })?
        }
    };

    let p = DVec2::from(input.reference_point);
    let (ends0, ends1) = spine
        .curve_ends
        .unwrap_or((l0 + u * span.0, l0 + u * span.1));
    // 支点は基準点から遠いほうの端(背は支点まわりに回り、自由端が基準点へ向かう)
    let (pivot, tip) = if (p - ends0).length() >= (p - ends1).length() {
        (ends0, ends1)
    } else {
        (ends1, ends0)
    };
    if (tip - pivot).length() <= EPS {
        return Err(format!(
            "{name}の開く折り目の長さが0です。中心線を引き直してください"
        ));
    }
    if (p - pivot).length() <= EPS {
        return Err(format!(
            "{name}のつぶす方向を示す点が支点と同じ位置です。つぶす先の点を指してください"
        ));
    }
    let s_dir = (tip - pivot).normalize();
    let c_dir = (p - pivot).normalize();
    let alpha = s_dir.perp_dot(c_dir).atan2(s_dir.dot(c_dir));

    let same_side = if spine.curve_ends.is_some() {
        spine_side_links(faces, &spine.edges, &spine.pairs)
    } else {
        Vec::new()
    };
    let (near, far) = split_by_spine(&flap, &spine.pairs, &same_side, name, &mut warnings);
    // 奥の半分がフラップの外の紙とつながっていると、層まるごと回すと必ずそこで裂ける。
    // 実際の紙では両側とも二等分線Mで折り返される(予備基本形の袋を開く動き)
    let anchored = spine.curve_ends.is_none()
        && !far.is_empty()
        && anchored_outside(
            cp,
            faces,
            state,
            &far,
            &flap,
            &spine.edges,
            SpineAxis {
                origin: l0,
                direction: u,
            },
        );
    let open = if input.open_to_back.unwrap_or(false) {
        FoldDirection::Down
    } else {
        FoldDirection::Up
    };
    let parts = if spine.curve_ends.is_some() {
        let polys = flap_polygons(cp, faces, state, &state.order);
        curved_squash_parts(CurvedSquashInput {
            cp,
            faces,
            pairs: &spine.pairs,
            state,
            polygons: &polys,
            motion: SquashMotion {
                near: &near,
                far: &far,
                pivot,
                spine_direction: s_dir,
                fold_angle: alpha,
                reach: (tip - pivot).length(),
                open,
            },
        })
    } else {
        squash_parts(StraightSquashInput {
            flap: &flap,
            motion: SquashMotion {
                near: &near,
                far: &far,
                pivot,
                spine_direction: s_dir,
                fold_angle: alpha,
                reach: (tip - pivot).length(),
                open,
            },
            closing_direction: c_dir,
            anchored,
        })
    };
    let mut res = flat_motion(
        cp,
        faces,
        state,
        &FlatMotionInput {
            parts,
            kind: TechniqueKind::Squash,
        },
    )?;
    if anchored {
        // 動きが変わったことを手順の注記で伝える(奥の紙の行き先が別物になる)
        res.step.note = format!(
            "{name}: 奥の紙が外の紙とつながっているため、奥側も手前と同じ折り線で折り返しました"
        );
    }
    warnings.append(&mut res.warnings);
    res.warnings = warnings;
    Ok(res)
}

/// 花弁折り(フラップの先端を持ち上げ、両側の縁を中心線に沿わせる)。
///
/// `line` は**中心線**: 持ち上げる先端と、その行き先が乗る畳み平面の直線。
/// `reference_point` は**持ち上げる先端の位置**: フラップが中心線の向きに占める
/// 範囲の両端のうち、この点に近いほうを先端とする(先端は反対の端の側へ回る)。
///
/// 動きは1回の [`flat_motion`] で表す。折り線は3本:
///
/// - 先端から出る**斜め2本**: 先端で「中心線」と「フラップの縁」がなす角の
///   二等分線。ここで折ると両側の縁が中心線にぴったり重なる(花弁折りの要)
/// - **ちょうつがい1本**: ここで先端側が折り返り、先端が反対の端へ持ち上がる。
///   位置は左右の斜め線が**フラップの外へ出る点**を結んで決める([`petal_hinge`])。
///   実際の紙でも斜めの折り目はフラップの縁で止まり、そこを結んだ線がちょうつがいに
///   なる。左右で止まり点までの距離が同じなら中心線に直交する線、違えば斜めの線に
///   なる(左右の縁の長さが違うフラップでも実際の紙では折れるので、平均で1本に
///   丸めない)。止まり点がフラップの境目に乗るので、そこで折り目が3本だけになって
///   平らに畳めなくなることがない
///
/// 紙の動き(3つの [`MotionPart`]):
///
/// - 両側の羽(斜め線の外側でちょうつがいの手前): 斜め線→ちょうつがいの鏡映2回
///   =回転。縁が中心線へ寄りながら持ち上がる
/// - 中央のくさび(斜め2本の間でちょうつがいの手前): ちょうつがいでの鏡映
/// - ちょうつがいの向こう側の紙は動かない
///
/// 持ち上げた紙は**袋ごと**に、その袋のいちばん外側の層の隣へ置き
/// (`open_to_back` が `Some(true)` なら向こう側)、中央のくさびを羽の外側に置く
/// (羽は先に中心へ折られてから一緒に裏返るので、上下が入れ替わる)。
/// 実際の紙では袋を1つずつ折るので、持ち上げた紙はその袋の中に留まり、
/// 別の袋の紙をまたがない。袋の切れ目は**中心線に乗った折り目**(袋の口を
/// 閉じている背)で、そこを渡らずに層をたどったまとまりが1つの袋になる
/// ([`petal_pockets`])。重なり全体の外側へまとめて回すと、袋がいくつも重なった
/// フラップ(カエルの基本形)で袋の紙が入り混じり、出来上がった先を1本ずつ
/// つまめなくなる。
/// 回す側を手前に決め打ちしないのは、実際の紙ではどちらへも折れるため
/// (鶴の基本形は前面と背面に1回ずつ花弁折りする)。
///
/// 層の数の偶奇やフラップの形は仮定せず、選んだ層すべてが同じように動く。
/// Errにするのは幾何的に決められない入力(退化した中心線・見つからない層・
/// 中心線の向きに広がりの無いフラップ・両側に紙の無いフラップ)だけで、
/// 紙が裂ける指定は警告して続ける。
pub fn petal(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &TechniqueInput,
) -> Result<FoldThroughResult, String> {
    let name = "花弁折り";
    let (l0, l1) = line_points(input.line)?;
    let u = (l1 - l0).normalize();
    let flap = flap_in_layer_order(faces, state, &input.flap, name)?;

    let mut warnings: Vec<String> = Vec::new();
    let span = flap_span_along(cp, faces, state, &flap, l0, u).ok_or_else(|| {
        format!("{name}のフラップが中心線の向きに広がっていません。中心線を引き直してください")
    })?;
    let p = DVec2::from(input.reference_point);
    let (e0, e1) = (l0 + u * span.0, l0 + u * span.1);
    let (tip, far) = if (p - e0).length() <= (p - e1).length() {
        (e0, e1)
    } else {
        (e1, e0)
    };
    if (far - tip).length() <= EPS {
        return Err(format!(
            "{name}の先端と行き先が同じ位置です。中心線を引き直してください"
        ));
    }
    let d = (far - tip).normalize();

    // 先端から見た両側の縁(中心線となす角と、その縁の長さ)
    let (right, left) = flap_sides(cp, faces, state, &flap, tip, d);
    if right.is_none() && left.is_none() {
        return Err(format!(
            "{name}の先端の両側に紙がありません。中心線と先端の位置を確かめてください"
        ));
    }
    // 左右の縁の長さが違っても折れる(ちょうつがいが斜めになるだけ)。
    // 平らに畳めないのは、縁が中心線の横まで倒れて二等分線とちょうつがいの
    // 交点が先端から無限に遠ざかる場合(角が直角に近づくと cos が0に近づく)
    if [right, left]
        .iter()
        .filter_map(|s| s.map(|(a, _)| a))
        .any(|a| a.abs() >= std::f64::consts::FRAC_PI_2 - ANGLE_EPS)
    {
        warnings.push(format!(
            "この{name}では、先端の紙が中心線の横まで広がっています。折り上がりが平らにならないことがあります(指定のまま続行します)"
        ));
    }

    // 羽のところでフラップとつながっている「フラップ外の層」。その折り目は
    // 花弁折りで開く(角0°)ので、相手の羽も中心線へ寄せないと紙が裂ける。
    // ちょうつがいは、縁を中心線へ寄せる折り目(二等分線)がフラップの外へ出る点を
    // 通る。紙の外へ出る点が読めないときだけ、縁の長さからの当て(従来の値)を使う
    let polys = flap_polygons(cp, faces, state, &flap);
    let (right_stop, left_stop, guessed) = {
        let mut guessed = false;
        let mut stop = |s: FlapSide| {
            s.map(|(ang, reach)| {
                let along = match ray_exit(&polys, tip, rotate(d, ang * 0.5)) {
                    Some(t) => t,
                    None => {
                        guessed = true;
                        reach / (ang * 0.5).cos().max(EPS)
                    }
                };
                (ang, along)
            })
        };
        let (r, l) = (stop(right), stop(left));
        (r, l, guessed)
    };
    if guessed {
        warnings.push(format!(
            "この{name}では、斜めの折り目がフラップの外へ出る点を読めませんでした。ちょうつがいの位置を縁の長さから見積もっています(指定のまま続行します)"
        ));
    }
    let geometry = PetalGeometry {
        tip,
        center_direction: d,
        hinge: petal_hinge(tip, d, right_stop, left_stop),
    };
    let selected_layers = PetalLayerSelection {
        cp,
        faces,
        state,
        flap: &flap,
    };
    let sides: Vec<PetalWing> = [right, left]
        .into_iter()
        .flatten()
        .map(|(ang, reach)| {
            let neighbors = wing_neighbors(
                &selected_layers,
                WingGeometry {
                    petal: geometry,
                    angle: ang,
                },
            );
            PetalWing {
                angle: ang,
                reach,
                neighbors,
            }
        })
        .collect();

    let open = if input.open_to_back.unwrap_or(false) {
        FoldDirection::Down
    } else {
        FoldDirection::Up
    };
    let pockets = petal_pockets(cp, faces, state, &flap, l0, u);
    let parts = petal_parts(PetalPartsInput {
        pockets: &pockets,
        polygons: &polys,
        geometry,
        wings: &sides,
        open,
    });
    // どの部分にも入らなかった層は動かない。片側だけの層が反対の羽から外れるのは
    // 普通のことだが、指定した層が1つの部分にも入らないのは指定の誤りなので伝える
    // (層を選ぶ側で黙って落とすと、誤った指定が無反応になってしまう)
    let mut idle: Vec<FaceId> = flap
        .iter()
        .copied()
        .filter(|id| !parts.iter().any(|p| p.layers.contains(id)))
        .collect();
    if !idle.is_empty() {
        idle.sort_unstable();
        let list: Vec<String> = idle.iter().map(|id| id.to_string()).collect();
        warnings.push(format!(
            "この{name}では、指定した層 {} が折り線の手前側に掛かっていないため動きません(指定のまま続行します)",
            list.join(", ")
        ));
    }
    let mut res = flat_motion(
        cp,
        faces,
        state,
        &FlatMotionInput {
            parts,
            kind: TechniqueKind::Petal,
        },
    )?;
    warnings.append(&mut res.warnings);
    res.warnings = warnings;
    Ok(res)
}

/// 沈め折り(open sink)。フラップの先端(角)を内側へ押し込む。
///
/// `line` は**沈める折り線**、`reference_point` は**押し込む先端側**を示す点
/// (この点のある側の紙が沈む)。
///
/// 動きは1回の [`flat_motion`] で表す。紙は1mmも動かない:
///
/// - 折り線の先端側を領域(半平面)にとる。境界線が各層のCPへ引き戻されて
///   新しい折り線になる = **(a) 折り線で各層を分割**
/// - 変換は [`MotionTransform::Stay`](紙は動かない)。沈め折りは畳んだ形を
///   変えず、重なりの内と外を入れ替える遷移だから
/// - `reverse_layers: Some(true)` で領域の中の重なり順だけを逆にする
///   = **(c) 層順序を内外反転して再挿入**(先端が袋の中へ入れ子になる)
/// - 山谷は [`flat_motion`] が新しい重なり順から決め直すので、先端側の
///   折り目は自動的に反転する = **(b) 先端側の山谷を反転**
///
/// `flap` が空なら折り線の先端側に掛かる全ての層を沈める(普通の沈め折り)。
/// 一部の層だけを選べば「部分的な沈め折り」になる。層の数の偶奇や先端の形は
/// 仮定しない。Errにするのは幾何的に決められない入力(退化した折り線・
/// 折り線の上にある基準点・見つからない層)だけで、紙が裂ける指定や
/// 山谷と重なり順の食い違いは警告して続ける。
pub fn open_sink(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &TechniqueInput,
) -> Result<FoldThroughResult, String> {
    let name = "沈め折り";
    let (l0, l1) = line_points(input.line)?;
    let u = (l1 - l0).normalize();
    let p = DVec2::from(input.reference_point);
    if u.perp_dot(p - l0).abs() <= EPS {
        return Err(format!(
            "{name}の沈める側を示す点が折り線の上にあります。押し込む先端側の点を指してください"
        ));
    }
    // 空の指定は「先端側に掛かる全ての層」。flat_motion が領域に掛からない層を
    // 自分で外すので、ここでは存在の検証だけしておく。
    let layers = if input.flap.is_empty() {
        Vec::new()
    } else {
        flap_in_layer_order(faces, state, &input.flap, name)?
    };

    flat_motion(
        cp,
        faces,
        state,
        &FlatMotionInput {
            parts: vec![MotionPart {
                layers,
                region: vec![HalfPlane {
                    line: input.line,
                    inside_point: input.reference_point,
                }],
                transform: MotionTransform::Stay,
                turn: LayerTurn::Keep,
                reverse_layers: Some(true),
            }],
            kind: TechniqueKind::OpenSink,
        },
    )
}

/// ひだ寄せ(swivel fold)。フラップの縁を支点のまわりに寄せ、余った紙をひだにする。
///
/// `line` は**基準線**: 寄せる紙が今乗っている折り目(または紙の縁)の直線。
/// `reference_point` は**寄せる先**: 基準線の自由端が向かう点。この2つで
/// 「基準線」と「寄せ線(支点から寄せる先へ向かう直線)」の2本を指定したことになる。
///
/// 支点は、フラップが基準線の向きに占める範囲の両端のうち**寄せる先から遠いほう**。
/// 支点から見た「基準線の向き」から「寄せる先の向き」までの角をαとすると、
/// 動きは1回の [`flat_motion`] で表せる:
///
/// - **くさび**(基準線と二等分線Mに挟まれた領域)は、支点から角α/2の向きの直線Mで
///   折り返す。基準線に乗っていた縁がちょうど寄せ線へ重なる
/// - **基準線の向こう側の紙**(寄せる先と反対側)は、支点まわりに角αだけ回る
///   (鏡映2回=回転)。くさびの縁と同じ場所へ写るので、基準線でつながったまま折れる
/// - Mの向こう(寄せる先の側)の紙は動かない。折り返したくさびはその上(または下)へ
///   回り、層が重なる(**層併合**)
/// - 折り線は基準線とMの2本で、支点で出会う(単純な折りを2回するのと同じだけの
///   折り線が、1回の動きで入る)
///
/// 折り返したくさびを重なりのどちら側へ入れるかは `open_to_back` で選ぶ
/// (既定は手前=いちばん上)。`flap` が空なら領域に掛かる全ての層を寄せる。
/// 層の数の偶奇やフラップの形は仮定しない。Errにするのは幾何的に決められない入力
/// (退化した基準線・見つからない層・基準線の向きに広がりの無いフラップ・
/// 支点と重なる寄せ先・基準線の上にある寄せ先)だけ。
pub fn swivel(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &TechniqueInput,
) -> Result<FoldThroughResult, String> {
    let name = "ひだ寄せ";
    let (l0, l1) = line_points(input.line)?;
    let u = (l1 - l0).normalize();
    let flap = flap_or_all(faces, state, &input.flap, name)?;
    let span = flap_span_along(cp, faces, state, &flap, l0, u).ok_or_else(|| {
        format!("{name}のフラップが基準線の向きに広がっていません。基準線を引き直してください")
    })?;

    let p = DVec2::from(input.reference_point);
    let (e0, e1) = (l0 + u * span.0, l0 + u * span.1);
    // 支点は寄せる先から遠いほうの端(自由端が寄せる先へ向かって回る)
    let (pivot, tip) = if (p - e0).length() >= (p - e1).length() {
        (e0, e1)
    } else {
        (e1, e0)
    };
    let reach = (tip - pivot).length();
    if reach <= EPS {
        return Err(format!(
            "{name}のフラップが基準線の向きに広がっていません。基準線を引き直してください"
        ));
    }
    if (p - pivot).length() <= EPS {
        return Err(format!(
            "{name}の寄せる先が支点と同じ位置です。寄せたい先の点を指してください"
        ));
    }
    let s_dir = (tip - pivot).normalize();
    let c_dir = (p - pivot).normalize();
    let alpha = s_dir.perp_dot(c_dir).atan2(s_dir.dot(c_dir));
    if alpha.abs() <= ANGLE_EPS {
        return Err(format!(
            "{name}の寄せる先が基準線の上にあります。基準線から離れた点を指してください"
        ));
    }

    let m_dir = rotate(s_dir, alpha * 0.5);
    let m_line = [[pivot.x, pivot.y], [pivot.x + m_dir.x, pivot.y + m_dir.y]];
    // くさび(基準線とMの間)の内側を示す点と、その反対側(基準線の向こう)の点
    let inside = pivot + rotate(s_dir, alpha * 0.25) * (reach * 0.5);
    let beyond = reflect_across_line(inside, l0, l1);
    let base_line = [[pivot.x, pivot.y], [pivot.x + s_dir.x, pivot.y + s_dir.y]];
    // 空の指定は「領域に掛かる全ての層」。層を並べ直して渡すと、掛からない層に
    // ついて余計な警告が出る
    let layers = if input.flap.is_empty() {
        Vec::new()
    } else {
        flap
    };
    let open = if input.open_to_back.unwrap_or(false) {
        FoldDirection::Down
    } else {
        FoldDirection::Up
    };
    let mut res = flat_motion(
        cp,
        faces,
        state,
        &FlatMotionInput {
            parts: vec![
                MotionPart {
                    layers: layers.clone(),
                    region: vec![
                        HalfPlane {
                            line: input.line,
                            inside_point: [inside.x, inside.y],
                        },
                        HalfPlane {
                            line: m_line,
                            inside_point: [inside.x, inside.y],
                        },
                    ],
                    transform: MotionTransform::Reflect(vec![m_line]),
                    turn: LayerTurn::Outside(open),
                    reverse_layers: None,
                },
                MotionPart {
                    layers,
                    region: vec![HalfPlane {
                        line: input.line,
                        inside_point: [beyond.x, beyond.y],
                    }],
                    // 鏡映2回=支点まわりの角αの回転(基準線→M の順に掛ける)
                    transform: MotionTransform::Reflect(vec![base_line, m_line]),
                    turn: LayerTurn::Keep,
                    reverse_layers: None,
                },
            ],
            kind: TechniqueKind::Swivel,
        },
    )?;
    if alpha.abs() >= std::f64::consts::PI - ANGLE_EPS {
        res.warnings.insert(
            0,
            format!(
                "この{name}では、寄せる先が基準線の真後ろにあります。くさびが紙の全体に広がるので、寄せ先の点を確かめてください(指定のまま続行します)"
            ),
        );
    }
    Ok(res)
}

/// ねじり折り(twist fold)。中央の多角形を回し、周りにひだを作る。
///
/// # 入力の決め方(設計)
///
/// 中央多角形は2通りに指せる。どちらでも `reference_point` = **回転量を示す点**で、
/// 中心から見た「1辺目の中点の向き」から「この点の向き」までの角がねじる角αになる。
///
/// - `polygon` を渡す = **頂点を順に並べた任意の多角形**(3点以上)。辺ごとに
///   長さの違う多角形(不等辺三角形・不等辺四角形など)もそのまま折れる。
///   中心は `center`(省略時は選んだ層の重心)
/// - `polygon` が空(従来の指し方) = `line` を**中央多角形の1辺**(2点は辺の両端)
///   とし、中心のまわりに回して正多角形を作る。辺が中心のまわりに張る角から
///   辺の数 n = 2π/∠ を決める
///
/// 中心は `center` で明示できる(省略時は選んだ層が畳み平面で占める範囲の重心)。
///
/// # 紙の動き(2n+1個の [`MotionPart`])
///
/// - **中央**(多角形の内側): 中心まわりの角αの回転
/// - **ひだ**(辺kの外側): 回転後の辺で折り返す(鏡映)。中央とは辺kでつながったまま
/// - **腕**(頂点の外側): ひだと折り目でつながるように決まる等長変換。ひだが
///   紙の縁まで伸びて腕どうしを切り離すので、腕は1つずつ別に動ける
///
/// 各頂点は「多角形の辺2本+ひだの折り線2本」の4本が集まる点になり、平らに畳める。
/// 層の数の偶奇やフラップの形・多角形の辺の数や長さは仮定しない。Errにするのは
/// 幾何的に決められない入力(退化した辺・見つからない層・中心と重なる辺の端・
/// 中央多角形が作れない角・回転量が0・頂点が3つ未満や重なった多角形)だけで、
/// 紙が裂ける指定・中心が多角形の外にある指定は警告して続ける。
pub fn twist(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &TechniqueInput,
) -> Result<FoldThroughResult, String> {
    let name = "ねじり折り";
    let flap = flap_or_all(faces, state, &input.flap, name)?;
    let center = match input.center {
        Some(c) => DVec2::from(c),
        None => flap_centroid(cp, faces, state, &flap)
            .ok_or_else(|| format!("{name}の中心が決められません。層を選び直してください"))?,
    };
    let mut warnings: Vec<String> = Vec::new();
    // `mid` は「ねじる角αを測る起点」= 1辺目の中点
    let (v, mid) = match input.polygon.as_deref() {
        Some(pts) => {
            let v = polygon_vertices(pts, center, name, &mut warnings)?;
            let mid = (v[0] + v[1]) * 0.5;
            (v, mid)
        }
        None => {
            let (a, b) = line_points(input.line)?;
            let v = regular_polygon(a, b, center, name, &mut warnings)?;
            (v, (a + b) * 0.5)
        }
    };

    let rp = DVec2::from(input.reference_point) - center;
    let rm = mid - center;
    if rp.length() <= EPS {
        return Err(format!(
            "{name}の回転量を示す点が中心と同じ位置です。中心から離れた点を指してください"
        ));
    }
    let alpha = rm.perp_dot(rp).atan2(rm.dot(rp));
    if alpha.abs() <= ANGLE_EPS {
        return Err(format!(
            "{name}のねじる角が0です。回転量を示す点をずらしてください"
        ));
    }

    let open = if input.open_to_back.unwrap_or(false) {
        FoldDirection::Down
    } else {
        FoldDirection::Up
    };
    let parts = twist_parts(&flap, &input.flap, center, &v, alpha, open);
    let mut res = flat_motion(
        cp,
        faces,
        state,
        &FlatMotionInput {
            parts,
            kind: TechniqueKind::Twist,
        },
    )?;
    warnings.append(&mut res.warnings);
    res.warnings = warnings;
    Ok(res)
}

// ---------------------------------------------------------------------------
// 技法の作業場: fold_throughを繰り返しながら、面の由来・裏返りの偶奇を追う
// ---------------------------------------------------------------------------

/// 技法の途中経過。CPは複製の上で書き換え、成功したときだけ [`Session::finish`] で
/// 呼び出し側のCPへ反映する(原子性: 途中で失敗しても元のCPは変わらない)。
struct Session {
    cp: CreasePattern,
    faces: Vec<Face>,
    state: FlatState,
    /// 現在の面ID → 技法開始時のどの面の子孫か
    origin: HashMap<FaceId, FaceId>,
    /// 現在の面ID → 技法の中で裏返った回数の偶奇(trueなら裏返っている)
    flipped: HashMap<FaceId, bool>,
    drivers: Vec<DriverLine>,
    added: Vec<EdgeId>,
    warnings: Vec<String>,
}

impl Session {
    fn new(cp: &CreasePattern, faces: &[Face], state: &FlatState) -> Result<Session, String> {
        for f in faces {
            if !state.placements.contains_key(&f.id) {
                return Err(format!("面 {} の配置が平坦状態に見つかりません", f.id));
            }
        }
        Ok(Session {
            cp: cp.clone(),
            faces: faces.to_vec(),
            state: state.clone(),
            origin: faces.iter().map(|f| (f.id, f.id)).collect(),
            flipped: faces.iter().map(|f| (f.id, false)).collect(),
            drivers: Vec::new(),
            added: Vec::new(),
            warnings: Vec::new(),
        })
    }

    /// 1回の [`fold_through`]。戻り値は「この折りで動かなかった側の点を、
    /// 折った後の畳み平面座標へ移す等長変換」(次の折り線を渡すときに使う)。
    ///
    /// `targets` が `None` なら可動側に掛かる全ての層が対象。
    ///
    /// 途中の折りで出る「紙が裂けます」と補助線昇格の警告は捨てる。技法は複数回の折りで
    /// 1つの形を作るため、1回目だけを見ると必ず層のつながりが切れて見える
    /// (最終形での裂けは [`Session::tear_warnings`] で改めて調べる)。補助線は技法が
    /// 予定した折り筋として使うため、利用者向けの昇格通知は技法の警告に含めない。
    fn fold(
        &mut self,
        line: [[f64; 2]; 2],
        keep_side_point: [f64; 2],
        targets: Option<&[FaceId]>,
        direction: FoldDirection,
    ) -> Result<Isometry2, String> {
        let old_cp = self.cp.clone();
        let old_faces = self.faces.clone();
        let old_state = self.state.clone();
        let res = fold_through(
            &mut self.cp,
            &old_faces,
            &old_state,
            &FoldThroughInput {
                line,
                keep_side_point,
                target_layers: targets.map(<[FaceId]>::to_vec),
                direction,
            },
        )?;

        let (l0, l1) = line_points(line)?;
        let u = (l1 - l0).normalize();
        let keep_sign = u.perp_dot(DVec2::from(keep_side_point) - l0).signum();
        let signed = |q: DVec2| keep_sign * u.perp_dot(q - l0);
        let refl = Isometry2::reflection(l0, l1);

        let new_faces = extract_faces(&self.cp);
        let mut origin: HashMap<FaceId, FaceId> = HashMap::with_capacity(new_faces.len());
        let mut flipped: HashMap<FaceId, bool> = HashMap::with_capacity(new_faces.len());
        // 動かなかった側の平面座標のずれ(根面をそろえ直す変換)。
        // 動いた面からも鏡映を打ち消せば同じものが求まる。
        let mut plane_map: Option<Isometry2> = None;
        for nf in &new_faces {
            let r = representative_point(&self.cp, nf);
            let parent = old_faces.iter().find(|f| point_in_face(&old_cp, f, r));
            let Some(pf) = parent else {
                // 親を特定できない面(fold_throughが警告済み)。由来を引き継げないので
                // 「技法の外の紙」として扱う
                origin.insert(nf.id, nf.id);
                flipped.insert(nf.id, false);
                continue;
            };
            let ppl = old_state.placements[&pf.id];
            let is_target = targets.is_none_or(|t| t.contains(&pf.id));
            let moved = is_target && signed(ppl.apply(DVec2::from(r))) < -EPS;
            origin.insert(nf.id, self.origin[&pf.id]);
            flipped.insert(nf.id, self.flipped[&pf.id] != moved);
            if plane_map.is_none() {
                let m = res.state.placements[&nf.id].compose(&ppl.inverse());
                plane_map = Some(if moved { m.compose(&refl) } else { m });
            }
        }

        self.faces = new_faces;
        self.state = res.state;
        self.origin = origin;
        self.flipped = flipped;
        self.drivers.extend(res.step.drivers);
        self.added.extend(res.added_edges);
        self.warnings.extend(
            res.warnings
                .into_iter()
                .filter(|w| !w.contains(TEAR_MARK) && !w.starts_with(AUX_PROMOTION_WARNING_MARK)),
        );
        Ok(plane_map.unwrap_or_else(Isometry2::identity))
    }

    /// 入力のフラップ指定を検証する。空なら `None`(可動側の全ての層が対象)。
    fn resolve_flap(&self, flap: &[FaceId], name: &str) -> Result<Option<Vec<FaceId>>, String> {
        if flap.is_empty() {
            return Ok(None);
        }
        let mut out: Vec<FaceId> = Vec::with_capacity(flap.len());
        for &id in flap {
            if !self.faces.iter().any(|f| f.id == id) {
                return Err(format!(
                    "{name}の対象に指定された層 {id} が見つかりません。層を選び直してください"
                ));
            }
            if !out.contains(&id) {
                out.push(id);
            }
        }
        Ok(Some(out))
    }

    /// フラップの面を層順(下→上)に並べる。
    fn flap_in_layer_order(&self, flap: &[FaceId], name: &str) -> Result<Vec<FaceId>, String> {
        let set = self
            .resolve_flap(flap, name)?
            .ok_or_else(|| format!("{name}にはフラップ(重なった層)の指定が必要です"))?;
        Ok(self
            .state
            .order
            .iter()
            .copied()
            .filter(|id| set.contains(id))
            .collect())
    }

    /// 技法開始時の面の集合に対応する、現在の面(子孫)の一覧。
    fn descendants(&self, of: &[FaceId]) -> Vec<FaceId> {
        self.faces
            .iter()
            .filter(|f| self.origin.get(&f.id).is_some_and(|o| of.contains(o)))
            .map(|f| f.id)
            .collect()
    }

    /// ここまでの折りで裏返った(=動いた)面の一覧。
    fn moved_faces(&self) -> Vec<FaceId> {
        self.faces
            .iter()
            .filter(|f| self.flipped.get(&f.id).copied() == Some(true))
            .map(|f| f.id)
            .collect()
    }

    /// 対象候補のうち、折り線の可動側に掛かる面だけを残す。
    /// [`fold_through`] は掛からない層を警告付きで除外するので、技法の内部呼び出しでは
    /// あらかじめ絞って余計な警告を出さない。
    fn movable_targets(
        &self,
        line: [[f64; 2]; 2],
        keep_side_point: [f64; 2],
        candidates: Option<&[FaceId]>,
    ) -> Option<Vec<FaceId>> {
        let candidates = candidates?;
        let Ok((l0, l1)) = line_points(line) else {
            return Some(candidates.to_vec());
        };
        let u = (l1 - l0).normalize();
        let keep_sign = u.perp_dot(DVec2::from(keep_side_point) - l0).signum();
        let pos = vertex_positions(&self.cp);
        Some(
            candidates
                .iter()
                .copied()
                .filter(|id| {
                    let Some(f) = self.faces.iter().find(|f| f.id == *id) else {
                        return false;
                    };
                    let pl = self.state.placements[id];
                    f.vertices
                        .iter()
                        .filter_map(|v| pos.get(v))
                        .any(|&p| keep_sign * u.perp_dot(pl.apply(p) - l0) < -EPS)
                })
                .collect(),
        )
    }

    /// フラップの層を「先端を上へ回す層」と「下へ回す層」に塗り分ける。
    ///
    /// 折り線が横切る折り目(背)でつながった2つの層は、先端どうしがつながったまま
    /// 裏返る(これが「裏返して差し込む」形になる理由)。つながった相手とは必ず
    /// 反対向きに回るので、つながりの図を2色で塗り分ければ向きが決まる。
    /// 層の数を機械的に半分に割る方法と違い、奇数層のフラップや、重なりの一部だけを
    /// 選んだフラップでも紙のつながりどおりに折れる。
    ///
    /// 塗り分けの向きは、つながりのかたまりごとに「いちばん手前(上)の層」で決める:
    /// 中割り折りではその層の先端が下(層の内側)へ、かぶせ折りでは上(外側)へ回る。
    /// つながりの図が2色で塗り分けられない(奇数の輪になる)場合は、紙として
    /// 成り立たない指定なのでErr。
    fn split_flap_by_connection(
        &self,
        flap: &[FaceId],
        line: [[f64; 2]; 2],
        inside: bool,
        name: &str,
    ) -> Result<HashMap<FaceId, bool>, String> {
        let members: HashSet<FaceId> = flap.iter().copied().collect();
        // つながりの図: 折り線が横切る折り目を共有する層の組
        let mut adj: HashMap<FaceId, Vec<FaceId>> =
            flap.iter().map(|&id| (id, Vec::new())).collect();
        for (eid, fs) in faces_by_edge(&self.faces) {
            if fs.len() != 2 || !fs.iter().all(|id| members.contains(id)) {
                continue;
            }
            let Some(e) = self.cp.edges.iter().find(|e| e.id == eid) else {
                continue;
            };
            if !matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley) {
                continue;
            }
            if !self.edge_crosses_line(e.v0, e.v1, fs[0], line) {
                continue;
            }
            adj.get_mut(&fs[0]).expect("フラップの面").push(fs[1]);
            adj.get_mut(&fs[1]).expect("フラップの面").push(fs[0]);
        }

        // かたまりごとに2色で塗り分ける(層順で後ろ=手前の層から始める)
        let mut up: HashMap<FaceId, bool> = HashMap::with_capacity(flap.len());
        // 手前の層の先端は、中割りなら下(内側)・かぶせなら上(外側)へ回る
        let front_up = !inside;
        for &start in flap.iter().rev() {
            if up.contains_key(&start) {
                continue;
            }
            up.insert(start, front_up);
            let mut queue = vec![start];
            while let Some(cur) = queue.pop() {
                let cur_up = up[&cur];
                for &next in &adj[&cur] {
                    match up.get(&next) {
                        Some(&v) if v == cur_up => {
                            return Err(format!(
                                "この重なり方には{name}ができません(層のつながりが輪になっていて、先端の向きが決められません)。フラップの選び方か折り線を変えるか、手動の折り操作で代替してください"
                            ));
                        }
                        Some(_) => {}
                        None => {
                            up.insert(next, !cur_up);
                            queue.push(next);
                        }
                    }
                }
            }
        }
        Ok(up)
    }

    /// 辺(CP座標)が畳み平面の折り線に横切られているか(両端が線の反対側にあるか)。
    /// `on` はこの辺を境界に持つ面(その配置で畳み平面へ写す)。
    fn edge_crosses_line(
        &self,
        v0: VertexId,
        v1: VertexId,
        on: FaceId,
        line: [[f64; 2]; 2],
    ) -> bool {
        let Ok((l0, l1)) = line_points(line) else {
            return false;
        };
        let u = (l1 - l0).normalize();
        let pos = vertex_positions(&self.cp);
        let (Some(&p0), Some(&p1)) = (pos.get(&v0), pos.get(&v1)) else {
            return false;
        };
        let pl = self.state.placements[&on];
        let d0 = u.perp_dot(pl.apply(p0) - l0);
        let d1 = u.perp_dot(pl.apply(p1) - l0);
        (d0 > EPS && d1 < -EPS) || (d0 < -EPS && d1 > EPS)
    }

    /// 面が折り線の両側に掛かっている(折り線が面を横切っている)か。
    fn crosses(&self, id: FaceId, line: [[f64; 2]; 2]) -> bool {
        let Ok((l0, l1)) = line_points(line) else {
            return false;
        };
        let u = (l1 - l0).normalize();
        let Some(f) = self.faces.iter().find(|f| f.id == id) else {
            return false;
        };
        let pl = self.state.placements[&id];
        let pos = vertex_positions(&self.cp);
        let (mut plus, mut minus) = (false, false);
        for p in f.vertices.iter().filter_map(|v| pos.get(v)) {
            let d = u.perp_dot(pl.apply(*p) - l0);
            if d > EPS {
                plus = true;
            } else if d < -EPS {
                minus = true;
            }
        }
        plus && minus
    }

    /// 動いた部分の中にある折り目の山谷を、動いた後の重なり順に合わせ直す。
    ///
    /// 折り目でつながった2面の上下は、その折り目を表から見たときの山谷で決まる
    /// (表向きの面から見て谷なら相手は上、山なら下)。技法で紙の一部が裏返り、
    /// 重なり順が入れ替わると、この関係が崩れることがある:
    ///
    /// - 段折り: 段の中の紙は「まとめて折り返された」ので、裏返ると同時に重なり順も
    ///   逆になる。差し引きで折り目の山谷は変わらない
    /// - 中割り折り・かぶせ折り: 先端は裏返るが、先端どうしの重なり順は保たれる
    ///   (だから「裏返して差し込む」形になる)。そのぶん折り目の山谷が入れ替わる
    ///
    /// どちらの場合も結果は同じ規則で決まるので、重なり順から必要な山谷を求めて
    /// 書き換え、変えた折り目はDriverLineとして記録する
    /// (記録しないと再生時に前の手順の角度のまま折られてしまう)。
    ///
    /// 層順序の並べ替え([`Session::reorder_reversed`])より後に呼ぶこと。
    fn fix_reversed_creases(&mut self) {
        let edge_faces = faces_by_edge(&self.faces);
        let pos = vertex_positions(&self.cp);
        let rank: HashMap<FaceId, usize> = self
            .state
            .order
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i))
            .collect();
        // (辺ID, あるべき線種)
        let mut fixes: Vec<(EdgeId, EdgeKind)> = Vec::new();
        for (eid, fs) in &edge_faces {
            if fs.len() != 2 {
                continue;
            }
            // 技法で動いた紙の中の折り目だけを見る
            if !fs
                .iter()
                .all(|id| self.flipped.get(id).copied() == Some(true))
            {
                continue;
            }
            let (a, b) = (fs[0], fs[1]);
            let (pa, pb) = (self.state.placements[&a], self.state.placements[&b]);
            // 折られていない(平らにつながったまま)折り目は上下を決めない
            if pa.mirrored == pb.mirrored {
                continue;
            }
            let (Some(&ra), Some(&rb)) = (rank.get(&a), rank.get(&b)) else {
                continue;
            };
            // 相手が上にあるのは「表向きから見て谷」のとき(裏返っていれば逆)
            let want = if (rb > ra) == pa.mirrored {
                EdgeKind::Mountain
            } else {
                EdgeKind::Valley
            };
            fixes.push((*eid, want));
        }
        for (eid, want) in fixes {
            let Some(e) = self.cp.edges.iter_mut().find(|e| e.id == eid) else {
                continue;
            };
            if !matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley) || e.kind == want {
                continue;
            }
            e.kind = want;
            let (v0, v1) = (e.v0, e.v1);
            let (Some(&a), Some(&b)) = (pos.get(&v0), pos.get(&v1)) else {
                continue;
            };
            push_driver_line(&mut self.drivers, a, b, angle_of(want));
        }
    }

    /// 裏返った先端を、それぞれが回った向きの側へ置き直す。
    ///
    /// [`fold_through`] は動いた層を山全体のいちばん上(Up)/いちばん下(Down)へ
    /// まとめる近似なので、そのままでは先端が紙の外側に出てしまう。実際には、
    /// 先端はもとの層(同じ面から分かれた残りの部分)のすぐ隣へ入る:
    /// 上へ回った先端はすぐ上、下へ回った先端はすぐ下。折り目の向き(山谷)と
    /// 重なり順の関係もこれで満たされる。
    ///
    /// `up` は面ID(技法開始時)→ 先端を上へ回すか(技法を始めた時点の畳み平面での向き)。
    /// `turned` は折りの途中で畳み平面ごと裏返った(重なり順の上下が入れ替わった)場合にtrue。
    fn reorder_tips(&mut self, up: &HashMap<FaceId, bool>, turned: bool) {
        let is_tip = |id: &FaceId| self.flipped.get(id).copied() == Some(true);
        // 先端を除いた並び(下→上)
        let base: Vec<FaceId> = self
            .state
            .order
            .iter()
            .copied()
            .filter(|id| !is_tip(id))
            .collect();
        let tips: Vec<FaceId> = self.state.order.iter().copied().filter(is_tip).collect();
        if tips.is_empty() {
            return;
        }
        let mut placed: HashSet<FaceId> = HashSet::new();
        let mut out: Vec<FaceId> = Vec::with_capacity(self.state.order.len());
        for &id in &base {
            let origin = self.origin[&id];
            let Some(&up_at_start) = up.get(&origin) else {
                out.push(id);
                continue;
            };
            let goes_up = up_at_start != turned;
            // 同じ面から分かれた先端を、回った向きの側の隣へ挟む
            let group: Vec<FaceId> = tips
                .iter()
                .copied()
                .filter(|t| !placed.contains(t) && self.origin[t] == origin)
                .collect();
            placed.extend(group.iter().copied());
            if goes_up {
                out.push(id);
                out.extend(group);
            } else {
                out.extend(group);
                out.push(id);
            }
        }
        // 相手が見つからなかった先端(面が丸ごと動いた場合)は、もとの並びの位置へ戻す
        for &t in &tips {
            if !placed.contains(&t) {
                let at = self
                    .state
                    .order
                    .iter()
                    .position(|&id| id == t)
                    .unwrap_or(0)
                    .min(out.len());
                out.insert(at, t);
            }
        }
        debug_assert_eq!(out.len(), self.state.order.len());
        self.state.order = out;
    }

    /// 最終形で紙が裂けている(動いた面とその隣の面のつながりが切れている)辺の警告。
    ///
    /// 技法の中で動いた面に接する辺だけを見る(技法より前の手順で入った裂けを
    /// 二重に報告しないため)。
    fn tear_warnings(&self) -> Vec<String> {
        let edge_faces = faces_by_edge(&self.faces);
        let pos = vertex_positions(&self.cp);
        let mut out: Vec<String> = Vec::new();
        for (eid, fs) in &edge_faces {
            if fs.len() != 2 {
                continue;
            }
            if !fs
                .iter()
                .any(|id| self.flipped.get(id).copied() == Some(true))
            {
                continue;
            }
            let Some(e) = self.cp.edges.iter().find(|e| e.id == *eid) else {
                continue;
            };
            let (Some(&p0), Some(&p1)) = (pos.get(&e.v0), pos.get(&e.v1)) else {
                continue;
            };
            let a = self.state.placements[&fs[0]];
            let b = self.state.placements[&fs[1]];
            if (a.apply(p0) - b.apply(p0)).length() > JOIN_EPS
                || (a.apply(p1) - b.apply(p1)).length() > JOIN_EPS
            {
                out.push(format!(
                    "折り目(辺ID {eid})の両側の紙が離れています。このままでは紙が裂けます(指定のまま続行します)"
                ));
            }
        }
        out
    }

    /// 折り上がりが紙の重なりとして成り立っているかを調べ、食い違いを警告にする。
    ///
    /// 折り目でつながった2面の上下は、その折り目を表から見たときの山谷で決まる
    /// (表向きの面から見て谷なら相手は上、山なら下)。技法で動いた紙に接する
    /// 折り目がこの関係を満たさないなら、展開図の山谷と記録した重なり順が食い違う
    /// = 展開図から折り直すと別の形になる。
    ///
    /// 断らずに警告にするのは「実際に折れるものを断らない」ため(「止めずに警告」原則。
    /// 紙が裂ける指定を [`fold_through`] が警告で通すのと同じ扱い)。多くは、重なりの
    /// 一部だけを選んだために紙が裂ける指定と一緒に出る。
    ///
    /// 技法で動いた面に接する折り目だけを見る(技法より前の手順で入った食い違いまで
    /// 数えない)。
    fn layer_consistency_warnings(&self, name: &str) -> Vec<String> {
        let rank: HashMap<FaceId, usize> = self
            .state
            .order
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i))
            .collect();
        let mut bad = 0usize;
        for (eid, fs) in faces_by_edge(&self.faces) {
            if fs.len() != 2 {
                continue;
            }
            let (a, b) = (fs[0], fs[1]);
            if !self.flipped.get(&a).copied().unwrap_or(false)
                && !self.flipped.get(&b).copied().unwrap_or(false)
            {
                continue;
            }
            let Some(e) = self.cp.edges.iter().find(|e| e.id == eid) else {
                continue;
            };
            if !matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley) {
                continue;
            }
            let (pa, pb) = (self.state.placements[&a], self.state.placements[&b]);
            // 折られていない(平らにつながったまま)折り目は上下を決めない
            if pa.mirrored == pb.mirrored {
                continue;
            }
            let (Some(&ra), Some(&rb)) = (rank.get(&a), rank.get(&b)) else {
                continue;
            };
            let want = if (rb > ra) == pa.mirrored {
                EdgeKind::Mountain
            } else {
                EdgeKind::Valley
            };
            if e.kind != want {
                bad += 1;
            }
        }
        if bad > 0 {
            return vec![format!(
                "この{name}では、折り目{bad}本の山谷と紙の重なり順が食い違います。このままでは展開図から折り直したときに形が変わります(指定のまま続行します)"
            )];
        }
        Vec::new()
    }

    /// 技法の結果をまとめ、CPを呼び出し側へ書き戻す。
    /// 紙が裂ける・山谷と重なり順が食い違うといった危うい折り上がりは警告にする
    /// (「止めずに警告」原則)。
    fn finish(
        mut self,
        kind: TechniqueKind,
        name: &str,
        cp: &mut CreasePattern,
    ) -> Result<FoldThroughResult, String> {
        let tears = self.tear_warnings();
        self.warnings.extend(tears);
        let inconsistent = self.layer_consistency_warnings(name);
        self.warnings.extend(inconsistent);
        let mut added = self.added;
        added.sort_unstable();
        added.dedup();
        added.retain(|id| self.cp.edges.iter().any(|e| e.id == *id));
        let layer_points = self.state.to_layer_points(&self.cp, &self.faces);
        let step = FoldStep {
            id: 0,
            kind,
            drivers: self.drivers,
            layer_order: Some(layer_points),
            alignment: None,
            finish_soft: None,
            note: String::new(),
        };
        *cp = self.cp;
        Ok(FoldThroughResult {
            state: self.state,
            added_edges: added,
            step,
            warnings: self.warnings,
            source_face_of: self.origin,
        })
    }
}

// ---------------------------------------------------------------------------
// 小さな道具
// ---------------------------------------------------------------------------

/// 入力のフラップ指定を検証し、層順(下→上)に並べる([`flat_motion`]の上に書いた
/// 技法の共通処理。指定が空・見つからない層はErrで断る)。
fn flap_in_layer_order(
    faces: &[Face],
    state: &FlatState,
    flap: &[FaceId],
    name: &str,
) -> Result<Vec<FaceId>, String> {
    let mut chosen: Vec<FaceId> = Vec::with_capacity(flap.len());
    for &id in flap {
        if !faces.iter().any(|f| f.id == id) {
            return Err(format!(
                "{name}の対象に指定された層 {id} が見つかりません。層を選び直してください"
            ));
        }
        if !chosen.contains(&id) {
            chosen.push(id);
        }
    }
    if chosen.is_empty() {
        return Err(format!(
            "{name}にはフラップ(重なった層)の指定が必要です。動かしたい重なりを選んでください"
        ));
    }
    Ok(state
        .order
        .iter()
        .copied()
        .filter(|id| chosen.contains(id))
        .collect())
}

/// フラップ指定を検証する。空なら「全ての層」(どの層に掛かるかは
/// [`flat_motion`] が領域との重なりで決める)。
fn flap_or_all(
    faces: &[Face],
    state: &FlatState,
    flap: &[FaceId],
    name: &str,
) -> Result<Vec<FaceId>, String> {
    if flap.is_empty() {
        return Ok(state.order.clone());
    }
    flap_in_layer_order(faces, state, flap, name)
}

/// つぶし折りで開く背。曲線は細分化された辺をまとめて保持する。
struct SquashSpine {
    /// 直線の背が入力線上で占める範囲。
    span: Option<(f64, f64)>,
    /// 背を挟んでつながる、選択フラップ内の面対。
    pairs: Vec<[FaceId; 2]>,
    /// 同じ背に属する辺。曲線の続きを外部固定と誤認しないためにも使う。
    edges: HashSet<EdgeId>,
    /// 曲線だった場合の両端(現在の畳み平面座標)。
    curve_ends: Option<(DVec2, DVec2)>,
}

/// 中心線に重なっている折り目(開く背)を探す。
///
/// 直線なら従来どおり入力線上の全断片を集める。入力線に乗る断片から45°以内で
/// 接線方向が最も滑らかにつながり、各中継点を直交するrulingが貫く折り辺列が
/// 見つかった場合は、曲線を近似した1本の背として全区間を集める。
fn spine_along(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    flap: &[FaceId],
    l0: DVec2,
    u: DVec2,
) -> SquashSpine {
    let pos = vertex_positions(cp);
    let by_edge = faces_by_edge(faces);
    let mut span: Option<(f64, f64)> = None;
    let mut pairs: Vec<[FaceId; 2]> = Vec::new();
    let mut straight_edges: Vec<EdgeId> = Vec::new();
    for (&eid, fs) in &by_edge {
        if fs.len() != 2 || !fs.iter().any(|id| flap.contains(id)) {
            continue;
        }
        let Some(e) = cp.edges.iter().find(|e| e.id == eid) else {
            continue;
        };
        if !matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let on = if flap.contains(&fs[0]) { fs[0] } else { fs[1] };
        let (Some(pl), Some(&p0), Some(&p1)) =
            (state.placements.get(&on), pos.get(&e.v0), pos.get(&e.v1))
        else {
            continue;
        };
        let (a, b) = (pl.apply(p0), pl.apply(p1));
        if u.perp_dot(a - l0).abs() > JOIN_EPS || u.perp_dot(b - l0).abs() > JOIN_EPS {
            continue;
        }
        let (ta, tb) = (u.dot(a - l0), u.dot(b - l0));
        span = Some(match span {
            None => (ta.min(tb), ta.max(tb)),
            Some((lo, hi)) => (lo.min(ta.min(tb)), hi.max(ta.max(tb))),
        });
        straight_edges.push(eid);
        if fs.iter().all(|id| flap.contains(id)) {
            pairs.push([fs[0], fs[1]]);
        }
    }
    straight_edges.sort_unstable();

    // 入力線上の断片をseedに、最長の曲がった連続辺列を選ぶ。
    let mut curved: Option<(Vec<EdgeId>, Vec<VertexId>)> = None;
    for &seed in &straight_edges {
        let Some((edge_ids, vertices)) = crease_chain(cp, seed, &pos) else {
            continue;
        };
        if !chain_is_curved(&vertices, &pos) {
            continue;
        }
        let replace = curved
            .as_ref()
            .is_none_or(|(best, _)| edge_ids.len() > best.len());
        if replace {
            curved = Some((edge_ids, vertices));
        }
    }

    let Some((edge_ids, vertices)) = curved else {
        return SquashSpine {
            span,
            pairs,
            edges: straight_edges.into_iter().collect(),
            curve_ends: None,
        };
    };

    let edges: HashSet<EdgeId> = edge_ids.iter().copied().collect();
    pairs.clear();
    for eid in &edge_ids {
        let Some(fs) = by_edge.get(eid) else {
            continue;
        };
        if fs.len() == 2 {
            pairs.push([fs[0], fs[1]]);
        }
    }
    let placed_end = |vid: VertexId, eid: EdgeId| -> Option<DVec2> {
        let fs = by_edge.get(&eid)?;
        let face = fs.first()?;
        Some(state.placements.get(face)?.apply(*pos.get(&vid)?))
    };
    let curve_ends = edge_ids
        .first()
        .zip(edge_ids.last())
        .and_then(|(&first, &last)| {
            Some((
                placed_end(*vertices.first()?, first)?,
                placed_end(*vertices.last()?, last)?,
            ))
        });

    SquashSpine {
        span,
        pairs,
        edges,
        curve_ends,
    }
}

/// seed辺の両端から、同じ山谷で接線方向が最も滑らかに続くものを追う。
///
/// 曲線ツールは各分割点へ接線と直交するrulingを両側へ引く。continuationの両端を
/// 除いた辺にその一直線がある頂点だけを中継点とすることで、曲線の端から角度の
/// 浅い通常の折り目へ誤って延長しない。後から交差で分割された点でもrulingは
/// その点を一直線に貫くので同じ判定になる。
fn crease_chain(
    cp: &CreasePattern,
    seed: EdgeId,
    pos: &HashMap<VertexId, DVec2>,
) -> Option<(Vec<EdgeId>, Vec<VertexId>)> {
    let seed_edge = cp.edges.iter().find(|e| e.id == seed)?;
    if !matches!(seed_edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
        return None;
    }
    let mut incident: HashMap<VertexId, Vec<EdgeId>> = HashMap::new();
    for e in &cp.edges {
        if e.kind == seed_edge.kind {
            incident.entry(e.v0).or_default().push(e.id);
            incident.entry(e.v1).or_default().push(e.id);
        }
    }
    for ids in incident.values_mut() {
        ids.sort_unstable();
    }

    let mut edges = std::collections::VecDeque::from([seed]);
    let mut vertices = std::collections::VecDeque::from([seed_edge.v0, seed_edge.v1]);
    let mut seen: HashSet<EdgeId> = HashSet::from([seed]);

    let mut extend = |front: bool,
                      edges: &mut std::collections::VecDeque<EdgeId>,
                      vertices: &mut std::collections::VecDeque<VertexId>| {
        loop {
            let (previous, at) = if front {
                (vertices[1], vertices[0])
            } else {
                let n = vertices.len();
                (vertices[n - 2], vertices[n - 1])
            };
            let (Some(&p_prev), Some(&p_at)) = (pos.get(&previous), pos.get(&at)) else {
                break;
            };
            let incoming = p_at - p_prev;
            if incoming.length() <= EPS {
                break;
            }
            let previous_edge = if front {
                edges[0]
            } else {
                edges[edges.len() - 1]
            };
            let mut candidates: Vec<(f64, EdgeId, VertexId)> = Vec::new();
            for &eid in incident.get(&at).map(Vec::as_slice).unwrap_or(&[]) {
                if seen.contains(&eid) {
                    continue;
                }
                let Some(e) = cp.edges.iter().find(|e| e.id == eid) else {
                    continue;
                };
                let next = if e.v0 == at { e.v1 } else { e.v0 };
                let Some(&p_next) = pos.get(&next) else {
                    continue;
                };
                let outgoing = p_next - p_at;
                if outgoing.length() <= EPS {
                    continue;
                }
                let bend = incoming
                    .perp_dot(outgoing)
                    .atan2(incoming.dot(outgoing))
                    .abs();
                if bend <= MAX_CURVE_BEND + ANGLE_EPS
                    && has_through_crease(cp, at, previous_edge, eid, pos)
                {
                    candidates.push((bend, eid, next));
                }
            }
            candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let Some((_, eid, next)) = candidates.first().copied() else {
                break;
            };
            seen.insert(eid);
            if front {
                edges.push_front(eid);
                vertices.push_front(next);
            } else {
                edges.push_back(eid);
                vertices.push_back(next);
            }
        }
    };
    extend(true, &mut edges, &mut vertices);
    extend(false, &mut edges, &mut vertices);
    Some((edges.into(), vertices.into()))
}

fn has_through_crease(
    cp: &CreasePattern,
    at: VertexId,
    previous_edge: EdgeId,
    next_edge: EdgeId,
    pos: &HashMap<VertexId, DVec2>,
) -> bool {
    let Some(&p_at) = pos.get(&at) else {
        return false;
    };
    let directions: Vec<DVec2> = cp
        .edges
        .iter()
        .filter(|e| {
            e.id != previous_edge
                && e.id != next_edge
                && e.kind != EdgeKind::Border
                && (e.v0 == at || e.v1 == at)
        })
        .filter_map(|e| {
            let other = if e.v0 == at { e.v1 } else { e.v0 };
            let direction = *pos.get(&other)? - p_at;
            (direction.length() > EPS).then(|| direction.normalize())
        })
        .collect();
    directions.iter().enumerate().any(|(i, &a)| {
        directions
            .iter()
            .skip(i + 1)
            .any(|&b| a.perp_dot(b).abs() <= CURVE_RULING_EPS && a.dot(b) < 0.0)
    })
}

fn chain_is_curved(vertices: &[VertexId], pos: &HashMap<VertexId, DVec2>) -> bool {
    vertices
        .windows(3)
        .filter(|v| {
            let (Some(&a), Some(&b), Some(&c)) = (pos.get(&v[0]), pos.get(&v[1]), pos.get(&v[2]))
            else {
                return false;
            };
            let (d0, d1) = (b - a, c - b);
            d0.length() > EPS
                && d1.length() > EPS
                && d0.perp_dot(d1).atan2(d0.dot(d1)).abs() > ANGLE_EPS
        })
        .take(2)
        .count()
        >= 2
}

/// 中心線に背が見つからないときの支点の当て(フラップが中心線の向きに占める範囲)。
fn flap_span_along(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    flap: &[FaceId],
    l0: DVec2,
    u: DVec2,
) -> Option<(f64, f64)> {
    let pos = vertex_positions(cp);
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for f in faces.iter().filter(|f| flap.contains(&f.id)) {
        let pl = state.placements.get(&f.id)?;
        for t in f
            .vertices
            .iter()
            .filter_map(|v| pos.get(v))
            .map(|&q| u.dot(pl.apply(q) - l0))
        {
            lo = lo.min(t);
            hi = hi.max(t);
        }
    }
    (lo < hi).then_some((lo, hi))
}

/// 曲線の背に沿って隣り合うfacetのうち、背を渡らず同じ側でrulingを共有する面対。
fn spine_side_links(
    faces: &[Face],
    spine_edges: &HashSet<EdgeId>,
    spine_pairs: &[[FaceId; 2]],
) -> Vec<[FaceId; 2]> {
    let spine_faces: HashSet<FaceId> = spine_pairs.iter().flatten().copied().collect();
    faces_by_edge(faces)
        .into_iter()
        .filter(|(edge, adjacent)| {
            !spine_edges.contains(edge)
                && adjacent.len() == 2
                && adjacent.iter().all(|face| spine_faces.contains(face))
        })
        .map(|(_, adjacent)| [adjacent[0], adjacent[1]])
        .collect()
}

/// フラップを背で2色に塗り分ける。戻り値は(手前寄り=下の層を含む側, 奥側)。
///
/// 背でつながった2層は開くと反対側へ分かれるので、つながりの図を2色で塗り分ければ
/// どちらの側へ行くかが決まる。曲線のrulingで隣り合うfacetは同じ色にして、区間ごとに
/// 層順が入れ替わっていても曲線の片側全体を同じ動きへまとめる。
/// (層の数を機械的に半分に割らないのは中割り折りと同じ)。
/// 塗り分けられない(奇数の輪になる)場合は、断らずに警告して続ける。
fn split_by_spine(
    flap: &[FaceId],
    pairs: &[[FaceId; 2]],
    same_side: &[[FaceId; 2]],
    name: &str,
    warnings: &mut Vec<String>,
) -> (Vec<FaceId>, Vec<FaceId>) {
    let mut color: HashMap<FaceId, bool> = HashMap::with_capacity(flap.len());
    let mut odd = false;
    for &start in flap {
        if color.contains_key(&start) {
            continue;
        }
        color.insert(start, false);
        let mut queue = vec![start];
        while let Some(cur) = queue.pop() {
            let cur_color = color[&cur];
            for (pr, opposite) in pairs
                .iter()
                .map(|pair| (pair, true))
                .chain(same_side.iter().map(|pair| (pair, false)))
            {
                let next = if pr[0] == cur {
                    pr[1]
                } else if pr[1] == cur {
                    pr[0]
                } else {
                    continue;
                };
                let expected = cur_color != opposite;
                match color.get(&next) {
                    Some(&v) => odd |= v != expected,
                    None => {
                        color.insert(next, expected);
                        queue.push(next);
                    }
                }
            }
        }
    }
    if odd {
        warnings.push(format!(
            "この{name}では、層のつながりが輪になっていて開く側を決めきれません(指定のまま続行します)"
        ));
    }
    flap.iter().partition(|id| !color[id])
}

/// 奥の半分が、フラップの外の紙と(中心線に乗っていない)折り目でつながっているか。
///
/// つながっていれば向こう端が固定されているので、層まるごと回すとそこで紙が裂ける
/// (中心線に乗っている折り目=これから開く背は、固定とは数えない)。
struct SpineAxis {
    origin: DVec2,
    direction: DVec2,
}

fn anchored_outside(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    group: &[FaceId],
    flap: &[FaceId],
    spine_edges: &HashSet<EdgeId>,
    axis: SpineAxis,
) -> bool {
    let SpineAxis {
        origin: l0,
        direction: u,
    } = axis;
    let pos = vertex_positions(cp);
    faces_by_edge(faces).into_iter().any(|(eid, fs)| {
        if fs.len() != 2 {
            return false;
        }
        let on = if group.contains(&fs[0]) && !flap.contains(&fs[1]) {
            fs[0]
        } else if group.contains(&fs[1]) && !flap.contains(&fs[0]) {
            fs[1]
        } else {
            return false;
        };
        let Some(e) = cp.edges.iter().find(|e| e.id == eid) else {
            return false;
        };
        if spine_edges.contains(&eid) {
            return false;
        }
        // 山谷だけでなく Aux(平らに開いた折り目)でも紙は外とつながっている。
        // 数えないと、そこで裂ける動きを選んでしまう
        if matches!(e.kind, EdgeKind::Border) {
            return false;
        }
        let (Some(pl), Some(&p0), Some(&p1)) =
            (state.placements.get(&on), pos.get(&e.v0), pos.get(&e.v1))
        else {
            return false;
        };
        let (a, b) = (pl.apply(p0), pl.apply(p1));
        u.perp_dot(a - l0).abs() > JOIN_EPS || u.perp_dot(b - l0).abs() > JOIN_EPS
    })
}

/// つぶし折りの動き(手前側=新しい折り線で折り返す / 奥側=層まるごと回る。
/// ただし `anchored` なら奥側も手前と同じ二等分線で折り返す)。
///
/// 曲線の背では区間ごとに隣接する面の現在配置が違う。手前側を二等分線で
/// 折り返したあとの配置へ、各区間の奥側facetを個別に開くことで、曲線頂点の
/// rulingを挟むfacetも共有辺上で同じ位置を保つ。
/// The common moving geometry and layer groups of a squash fold.
struct SquashMotion<'a> {
    near: &'a [FaceId],
    far: &'a [FaceId],
    pivot: DVec2,
    spine_direction: DVec2,
    fold_angle: f64,
    reach: f64,
    open: FoldDirection,
}

/// Inputs specific to a squash fold with a curved spine.
struct CurvedSquashInput<'a> {
    cp: &'a CreasePattern,
    faces: &'a [Face],
    pairs: &'a [[FaceId; 2]],
    state: &'a FlatState,
    polygons: &'a HashMap<FaceId, Vec<DVec2>>,
    motion: SquashMotion<'a>,
}

/// Inputs specific to a squash fold with a straight spine.
struct StraightSquashInput<'a> {
    flap: &'a [FaceId],
    motion: SquashMotion<'a>,
    closing_direction: DVec2,
    anchored: bool,
}

fn curved_squash_parts(input: CurvedSquashInput<'_>) -> Vec<MotionPart> {
    let CurvedSquashInput {
        cp,
        faces,
        pairs,
        state,
        polygons: polys,
        motion:
            SquashMotion {
                near,
                far,
                pivot,
                spine_direction: s_dir,
                fold_angle: alpha,
                reach,
                open,
            },
    } = input;
    if alpha.abs() <= ANGLE_EPS {
        let mut layers = near.to_vec();
        layers.extend(far.iter().copied());
        return vec![MotionPart::restack(layers, LayerTurn::Outside(open))];
    }

    let (sn, cs) = (alpha * 0.5).sin_cos();
    let m_dir = DVec2::new(s_dir.x * cs - s_dir.y * sn, s_dir.x * sn + s_dir.y * cs);
    let m_line = [[pivot.x, pivot.y], [pivot.x + m_dir.x, pivot.y + m_dir.y]];
    let inside = pivot + s_dir * reach;
    let reflected = Isometry2::reflection(pivot, pivot + m_dir);

    let near_set: HashSet<FaceId> = near.iter().copied().collect();
    let far_set: HashSet<FaceId> = far.iter().copied().collect();
    let mut far_open: HashMap<FaceId, (Isometry2, FaceId)> = HashMap::new();
    for pair in pairs {
        let (n, f) = if near_set.contains(&pair[0]) && far_set.contains(&pair[1]) {
            (pair[0], pair[1])
        } else if near_set.contains(&pair[1]) && far_set.contains(&pair[0]) {
            (pair[1], pair[0])
        } else {
            continue;
        };
        let (Some(near_placement), Some(far_placement)) =
            (state.placements.get(&n), state.placements.get(&f))
        else {
            continue;
        };
        // near ∘ far^-1: 奥facetの現在座標を、対応する手前facetの座標へ開く。
        let motion = near_placement.compose(&far_placement.inverse());
        far_open.entry(f).or_insert((motion, n));
    }

    let near_region = vec![HalfPlane {
        line: m_line,
        inside_point: [inside.x, inside.y],
    }];
    if !far_open.is_empty()
        && far_open.len() == far_set.len()
        && far_open
            .values()
            .all(|(opening, _)| opening.approx_eq(&Isometry2::identity(), JOIN_EPS))
    {
        // 背が既に0°で開いている曲線では、両側のfacetは1枚の連続した紙面にある。
        // 曲線facetを種に、二等分線の可動側で共有辺が実際につながる面へ同じ鏡映を
        // 伝える。選択したfacetだけを動かすと、その外周のrulingで紙が裂ける。
        let mut seeds = near.to_vec();
        seeds.extend(far.iter().copied());
        let moving = connected_layers_in_region(cp, faces, state, polys, &seeds, &near_region);
        return (!moving.is_empty())
            .then_some(MotionPart {
                layers: moving,
                region: near_region,
                transform: MotionTransform::Isometry(reflected),
                turn: LayerTurn::Outside(open),
                reverse_layers: None,
            })
            .into_iter()
            .collect();
    }
    let moving_near = layers_in_region(polys, near, &near_region);
    let mut parts = Vec::with_capacity(far_open.len() * 2 + usize::from(!moving_near.is_empty()));
    if !moving_near.is_empty() {
        parts.push(MotionPart {
            layers: moving_near,
            region: near_region,
            transform: MotionTransform::Isometry(reflected),
            turn: LayerTurn::Outside(open),
            reverse_layers: None,
        });
    }
    let mut far_ids: Vec<FaceId> = far_open.keys().copied().collect();
    far_ids.sort_by_key(|id| {
        state
            .order
            .iter()
            .position(|face| face == id)
            .unwrap_or(usize::MAX)
    });
    for id in far_ids {
        let (opening, anchor) = far_open[&id];
        // far側では「near側の二等分線」をopeningの逆で引き戻した線で分ける。
        // これにより、開くだけの部分と開いて折り返す部分が境界上で一致する。
        let inv = opening.inverse();
        let far_pivot = inv.apply(pivot);
        let far_axis = inv.apply(pivot + m_dir);
        let far_inside = inv.apply(inside);
        let other_side = reflect_across_line(inside, pivot, pivot + m_dir);
        let far_outside = inv.apply(other_side);
        let line = [[far_pivot.x, far_pivot.y], [far_axis.x, far_axis.y]];
        let moving_region = vec![HalfPlane {
            line,
            inside_point: [far_inside.x, far_inside.y],
        }];
        if !layers_in_region(polys, &[id], &moving_region).is_empty() {
            parts.push(MotionPart {
                layers: vec![id],
                region: moving_region,
                transform: MotionTransform::Isometry(reflected.compose(&opening)),
                turn: LayerTurn::Beside {
                    anchor,
                    direction: open,
                },
                reverse_layers: None,
            });
        }
        let stationary_region = vec![HalfPlane {
            line,
            inside_point: [far_outside.x, far_outside.y],
        }];
        if !layers_in_region(polys, &[id], &stationary_region).is_empty()
            && !opening.approx_eq(&Isometry2::identity(), JOIN_EPS)
        {
            parts.push(MotionPart {
                layers: vec![id],
                region: stationary_region,
                transform: MotionTransform::Isometry(opening),
                turn: LayerTurn::Beside {
                    anchor,
                    direction: open,
                },
                reverse_layers: None,
            });
        }
    }
    parts
}

fn squash_parts(input: StraightSquashInput<'_>) -> Vec<MotionPart> {
    let StraightSquashInput {
        flap,
        motion:
            SquashMotion {
                near,
                far,
                pivot,
                spine_direction: s_dir,
                fold_angle: alpha,
                reach,
                open,
            },
        closing_direction: c_dir,
        anchored,
    } = input;
    // 退化ケース: 背が向きを変えないので紙は動かない(重なり順と山谷だけが変わる)
    if alpha.abs() <= ANGLE_EPS {
        return vec![MotionPart::restack(flap.to_vec(), LayerTurn::Outside(open))];
    }
    let seg = |dir: DVec2| [[pivot.x, pivot.y], [pivot.x + dir.x, pivot.y + dir.y]];
    // 新しい折り線: 背の今の向きと行き先の角の二等分線(ここで折ると背が行き先へ向く)
    let (sn, cs) = (alpha * 0.5).sin_cos();
    let m_dir = DVec2::new(s_dir.x * cs - s_dir.y * sn, s_dir.x * sn + s_dir.y * cs);
    let m_line = seg(m_dir);
    let inside = pivot + s_dir * reach;
    if anchored {
        // 両側とも二等分線Mで折り返す。背は開かない(両側が同じように折り返るので
        // 角度が変わらず、背は中心線に乗った折り目として残る)。折り返した紙は
        // それぞれ元の層の隣へ入る(手前の紙は open の側、奥の紙はその反対側)
        let back = match open {
            FoldDirection::Up => FoldDirection::Down,
            FoldDirection::Down => FoldDirection::Up,
        };
        return [(near, open), (far, back)]
            .into_iter()
            .filter(|(layers, _)| !layers.is_empty())
            .map(|(layers, dir)| MotionPart {
                layers: layers.to_vec(),
                region: vec![HalfPlane {
                    line: m_line,
                    inside_point: [inside.x, inside.y],
                }],
                transform: MotionTransform::Reflect(vec![m_line]),
                turn: LayerTurn::Inside(dir),
                reverse_layers: None,
            })
            .collect();
    }
    let mut parts: Vec<MotionPart> = Vec::new();
    if !near.is_empty() {
        parts.push(MotionPart {
            layers: near.to_vec(),
            region: vec![HalfPlane {
                line: m_line,
                inside_point: [inside.x, inside.y],
            }],
            transform: MotionTransform::Reflect(vec![m_line]),
            turn: LayerTurn::Outside(open),
            reverse_layers: None,
        });
    }
    if !far.is_empty() {
        // 鏡映2回=角αの回転。背が開いて(角0°になって)行き先へ向く
        parts.push(MotionPart {
            layers: far.to_vec(),
            region: Vec::new(),
            transform: MotionTransform::Reflect(vec![m_line, seg(c_dir)]),
            turn: LayerTurn::Outside(open),
            reverse_layers: None,
        });
    }
    parts
}

/// 花弁折りの、先端から見たフラップの片側の縁: (中心線となす角(rad), 縁の長さ)。
/// 紙の無い側は `None`。
type FlapSide = Option<(f64, f64)>;

/// 先端から見たフラップの両側の縁。戻り値は(右回り側, 左回り側)。
///
/// 縁は「先端から見て中心線から最も開いた向きにある頂点」で決める(同じ角なら遠いほう)。
/// フラップの形も層の数も仮定しないための決め方。
fn flap_sides(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    flap: &[FaceId],
    tip: DVec2,
    d: DVec2,
) -> (FlapSide, FlapSide) {
    let pos = vertex_positions(cp);
    let (mut right, mut left): (FlapSide, FlapSide) = (None, None);
    for f in faces.iter().filter(|f| flap.contains(&f.id)) {
        let Some(pl) = state.placements.get(&f.id) else {
            continue;
        };
        for v in f.vertices.iter().filter_map(|v| pos.get(v)) {
            let q = pl.apply(*v) - tip;
            let r = q.length();
            if r <= EPS {
                continue;
            }
            let ang = d.perp_dot(q).atan2(d.dot(q));
            let slot = if ang > ANGLE_EPS {
                &mut left
            } else if ang < -ANGLE_EPS {
                &mut right
            } else {
                continue;
            };
            let better = match slot {
                None => true,
                Some((a0, r0)) => {
                    let (w, w0) = (ang.abs(), a0.abs());
                    w > w0 + ANGLE_EPS || ((w - w0).abs() <= ANGLE_EPS && r > *r0)
                }
            };
            if better {
                *slot = Some((ang, r));
            }
        }
    }
    (right, left)
}

/// 花弁折りのちょうつがい線を、左右の二等分線が**フラップの外へ出る点**から求める。
///
/// 実際の紙では、縁を中心線へ寄せる折り目(二等分線)はフラップの縁で止まる。
/// ちょうつがいはその2つの止まり点を結ぶ線で、ここで折ると縁がちょうど中心線へ
/// 寄り、持ち上げた紙は止まり点のまわりに回る(止まり点はフラップの境目に乗るので、
/// そこで折り目が3本になって畳めなくなることがない)。
/// 左右で止まり点までの距離が違えば、ちょうつがいは中心線に直交しない斜めの線になる。
///
/// 引数の `right`/`left` は(中心線となす角, 先端から止まり点までの距離)。
/// 片側にしか紙がない(または2つの点が重なる)ときは、その点を通る直交線とする。
fn petal_hinge(tip: DVec2, d: DVec2, right: FlapSide, left: FlapSide) -> [[f64; 2]; 2] {
    let cross = |(ang, along): (f64, f64)| tip + rotate(d, ang * 0.5) * along;
    let perpendicular = |q: DVec2| [[q.x, q.y], [q.x - d.y, q.y + d.x]];
    match (right.map(cross), left.map(cross)) {
        (Some(a), Some(b)) if (b - a).length() > EPS => [[a.x, a.y], [b.x, b.y]],
        (Some(a), _) => perpendicular(a),
        (None, Some(b)) => perpendicular(b),
        (None, None) => perpendicular(tip),
    }
}

/// フラップが畳み平面で占める多角形の一覧。
fn flap_polygons(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    flap: &[FaceId],
) -> HashMap<FaceId, Vec<DVec2>> {
    let pos = vertex_positions(cp);
    faces
        .iter()
        .filter(|f| flap.contains(&f.id))
        .filter_map(|f| {
            let pl = state.placements.get(&f.id)?;
            let poly = f
                .vertices
                .iter()
                .filter_map(|v| pos.get(v))
                .map(|&q| pl.apply(q))
                .collect();
            Some((f.id, poly))
        })
        .collect()
}

/// 多角形を半平面で切り取る(Sutherland–Hodgman)。
fn clip_polygon(poly: &[DVec2], inside: &dyn Fn(DVec2) -> f64) -> Vec<DVec2> {
    let mut out: Vec<DVec2> = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (da, db) = (inside(a), inside(b));
        if da >= 0.0 {
            out.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) && (db - da).abs() > f64::EPSILON {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

/// 領域(半平面の積)に紙が残る層だけを選ぶ。
///
/// 花弁折りは左右の羽と中央のくさびを別々の部分にするので、片側にしか無い層は
/// 反対側の羽に掛からない。それは指定の誤りではないので、警告を出さずに外す。
/// どの部分にも入らなかった層(=まったく動かない層)は、部分を組み立てたあとに
/// [`petal`] がまとめて警告するので、誤った層指定が無反応になることはない。
fn layers_in_region(
    polys: &HashMap<FaceId, Vec<DVec2>>,
    layers: &[FaceId],
    region: &[HalfPlane],
) -> Vec<FaceId> {
    layers
        .iter()
        .copied()
        .filter(|id| {
            let Some(poly) = polys.get(id) else {
                return true;
            };
            let mut cur = poly.clone();
            for hp in region {
                let inside = half_plane(hp.line, DVec2::from(hp.inside_point));
                cur = clip_polygon(&cur, &inside);
                if cur.len() < 3 {
                    return false;
                }
            }
            let area: f64 = (0..cur.len())
                .map(|i| cur[i].perp_dot(cur[(i + 1) % cur.len()]))
                .sum::<f64>()
                .abs();
            area * 0.5 > EPS * EPS
        })
        .collect()
}

/// 種となるfacetから、領域の内部に長さを持つ共有辺を渡ってつながる面を集める。
///
/// 曲線のrulingで細分化された紙面へ同じ等長変換を伝えるために使う。折り線となる
/// 領域境界上だけで接する面や、頂点1点で接するだけの面へは伝えない。
fn connected_layers_in_region(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    polys: &HashMap<FaceId, Vec<DVec2>>,
    seeds: &[FaceId],
    region: &[HalfPlane],
) -> Vec<FaceId> {
    let all: Vec<FaceId> = faces.iter().map(|face| face.id).collect();
    let eligible: HashSet<FaceId> = layers_in_region(polys, &all, region).into_iter().collect();
    let by_edge = faces_by_edge(faces);
    let face_by_id: HashMap<FaceId, &Face> = faces.iter().map(|face| (face.id, face)).collect();
    let positions = vertex_positions(cp);
    let mut reached: HashSet<FaceId> = seeds
        .iter()
        .copied()
        .filter(|face| eligible.contains(face))
        .collect();
    let mut queue: Vec<FaceId> = reached.iter().copied().collect();
    while let Some(face_id) = queue.pop() {
        let (Some(face), Some(placement)) =
            (face_by_id.get(&face_id), state.placements.get(&face_id))
        else {
            continue;
        };
        for &edge_id in &face.edges {
            let Some(adjacent) = by_edge.get(&edge_id) else {
                continue;
            };
            if adjacent.len() != 2 {
                continue;
            }
            let next = if adjacent[0] == face_id {
                adjacent[1]
            } else {
                adjacent[0]
            };
            if reached.contains(&next) || !eligible.contains(&next) {
                continue;
            }
            let Some(edge) = cp.edges.iter().find(|edge| edge.id == edge_id) else {
                continue;
            };
            let (Some(&p0), Some(&p1)) = (positions.get(&edge.v0), positions.get(&edge.v1)) else {
                continue;
            };
            if segment_has_region_length(placement.apply(p0), placement.apply(p1), region) {
                reached.insert(next);
                queue.push(next);
            }
        }
    }
    state
        .order
        .iter()
        .copied()
        .filter(|face| reached.contains(face))
        .collect()
}

fn segment_has_region_length(a: DVec2, b: DVec2, region: &[HalfPlane]) -> bool {
    if (b - a).length() <= EPS {
        return false;
    }
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for half_plane in region {
        let l0 = DVec2::from(half_plane.line[0]);
        let direction = DVec2::from(half_plane.line[1]) - l0;
        if direction.length() <= EPS {
            return false;
        }
        let direction = direction.normalize();
        let side = direction
            .perp_dot(DVec2::from(half_plane.inside_point) - l0)
            .signum();
        let da = side * direction.perp_dot(a - l0) - JOIN_EPS;
        let db = side * direction.perp_dot(b - l0) - JOIN_EPS;
        if da <= 0.0 && db <= 0.0 {
            return false;
        }
        if da <= 0.0 {
            lo = lo.max(-da / (db - da));
        } else if db <= 0.0 {
            hi = hi.min(da / (da - db));
        }
        if hi - lo <= EPS {
            return false;
        }
    }
    hi - lo > EPS
}

/// 多角形の内部(境界を含まない)に点があるか。
fn in_polygon(poly: &[DVec2], p: DVec2) -> bool {
    let mut inside = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    inside
}

/// 点が多角形の辺の上(EPS まで)にあるか。
fn on_edge(poly: &[DVec2], p: DVec2) -> bool {
    (0..poly.len()).any(|i| {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let e = b - a;
        let len2 = e.length_squared();
        if len2 <= EPS * EPS {
            return false;
        }
        let t = (e.dot(p - a) / len2).clamp(0.0, 1.0);
        (a + e * t).distance(p) <= EPS
    })
}

/// その点にフラップの紙があるか。
///
/// 多角形の内部にあれば紙。内部でなくても2枚以上の面の辺に乗っていれば、
/// そこは面と面の境目(折り目)なので紙があるとみなす。二等分線が既存の折り目に
/// 完全に乗ると、内部判定(境界を含まない)だけでは紙の途中で止まってしまい、
/// ちょうつがいが黙って旧式の当て値に落ちるため。
fn on_paper(polys: &HashMap<FaceId, Vec<DVec2>>, p: DVec2) -> bool {
    if polys.values().any(|poly| in_polygon(poly, p)) {
        return true;
    }
    polys.values().filter(|poly| on_edge(poly, p)).count() >= 2
}

/// 先端 `tip` から向き `dir` へ伸びる半直線が、フラップの紙の外へ出るまでの距離。
///
/// 二等分線の折り目が届く先(=ちょうつがいの通る点)。紙が無ければ `None`。
fn ray_exit(polys: &HashMap<FaceId, Vec<DVec2>>, tip: DVec2, dir: DVec2) -> Option<f64> {
    let mut ts: Vec<f64> = Vec::new();
    for poly in polys.values() {
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let e = b - a;
            let den = dir.perp_dot(e);
            if den.abs() <= EPS {
                continue;
            }
            let s = dir.perp_dot(tip - a) / den;
            let t = e.perp_dot(tip - a) / den;
            if (-EPS..=1.0 + EPS).contains(&s) && t > EPS {
                ts.push(t);
            }
        }
    }
    ts.sort_by(|a, b| a.partial_cmp(b).expect("有限の距離"));
    let mut prev = 0.0_f64;
    for t in ts {
        if t <= prev + EPS {
            continue;
        }
        let mid = tip + dir * ((prev + t) * 0.5);
        if !on_paper(polys, mid) {
            break;
        }
        prev = t;
    }
    (prev > EPS).then_some(prev)
}

/// 直線 `line` の `inside` 側を正とする符号付き距離(半平面の内外判定用)。
fn half_plane(line: [[f64; 2]; 2], inside: DVec2) -> impl Fn(DVec2) -> f64 {
    let l0 = DVec2::from(line[0]);
    let u = (DVec2::from(line[1]) - l0).normalize();
    let sign = u.perp_dot(inside - l0).signum();
    move |q: DVec2| sign * u.perp_dot(q - l0)
}

/// 羽の領域で、フラップの層と折り目でつながっている「フラップ外の層」。
///
/// 花弁折りでは、羽の外側の縁(フラップの層と隣の層をつないでいる折り目)が
/// **開く**。相手の層の羽も一緒に中心線へ寄せないと、そこで紙が裂ける。
/// The crease pattern and selected flap layers used to find wing neighbors.
struct PetalLayerSelection<'a> {
    cp: &'a CreasePattern,
    faces: &'a [Face],
    state: &'a FlatState,
    flap: &'a [FaceId],
}

/// The shared geometry of a petal fold.
#[derive(Clone, Copy)]
struct PetalGeometry {
    tip: DVec2,
    center_direction: DVec2,
    hinge: [[f64; 2]; 2],
}

/// The geometry of one wing relative to the shared petal geometry.
struct WingGeometry {
    petal: PetalGeometry,
    angle: f64,
}

/// One wing's extent and the non-flap layers it opens with.
struct PetalWing {
    angle: f64,
    reach: f64,
    neighbors: Vec<FaceId>,
}

/// The layer groups and geometry required to build all petal motion parts.
struct PetalPartsInput<'a> {
    pockets: &'a [Vec<FaceId>],
    polygons: &'a HashMap<FaceId, Vec<DVec2>>,
    geometry: PetalGeometry,
    wings: &'a [PetalWing],
    open: FoldDirection,
}

fn wing_neighbors(selection: &PetalLayerSelection<'_>, wing: WingGeometry) -> Vec<FaceId> {
    let PetalLayerSelection {
        cp,
        faces,
        state,
        flap,
    } = selection;
    let WingGeometry {
        petal:
            PetalGeometry {
                tip,
                center_direction: d,
                hinge,
            },
        angle: ang,
    } = wing;
    let pos = vertex_positions(cp);
    let k = rotate(d, ang * 0.5);
    let near = half_plane(hinge, tip);
    let planes: [&dyn Fn(DVec2) -> f64; 2] =
        [&near, &|q: DVec2| ang.signum() * k.perp_dot(q - tip)];
    let mut out: Vec<FaceId> = Vec::new();
    for (eid, fs) in faces_by_edge(faces) {
        if fs.len() != 2 {
            continue;
        }
        let (mine, other) = match (flap.contains(&fs[0]), flap.contains(&fs[1])) {
            (true, false) => (fs[0], fs[1]),
            (false, true) => (fs[1], fs[0]),
            _ => continue,
        };
        if out.contains(&other) {
            continue;
        }
        let Some(e) = cp.edges.iter().find(|e| e.id == eid) else {
            continue;
        };
        if !matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let (Some(pl), Some(&p0), Some(&p1)) =
            (state.placements.get(&mine), pos.get(&e.v0), pos.get(&e.v1))
        else {
            continue;
        };
        if segment_enters(pl.apply(p0), pl.apply(p1), &planes) {
            out.push(other);
        }
    }
    out
}

/// 線分が凸領域(符号付き距離の列。全て正なら内側)の内部を通るか。
fn segment_enters(p0: DVec2, p1: DVec2, planes: &[&dyn Fn(DVec2) -> f64]) -> bool {
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for plane in planes {
        let (a, b) = (plane(p0), plane(p1));
        let diff = b - a;
        if diff.abs() <= EPS {
            if a <= EPS {
                return false;
            }
            continue;
        }
        let t = -a / diff;
        if diff > 0.0 {
            lo = lo.max(t);
        } else {
            hi = hi.min(t);
        }
    }
    hi - lo > EPS
}

/// 中心線の向き `d` を角 `a` だけ回した向き。
fn rotate(d: DVec2, a: f64) -> DVec2 {
    let (s, c) = a.sin_cos();
    DVec2::new(d.x * c - d.y * s, d.x * s + d.y * c)
}

/// 花弁折りの動き。`sides` は側ごとの(中心線となす角, 一緒に開く隣の層)。
///
/// - フラップの羽(斜め線の外側・ちょうつがいの手前): 斜め線→ちょうつがいの鏡映2回
/// - 隣の層の羽: 斜め線の鏡映1回(縁を中心線へ寄せるだけ)。フラップの羽との
///   折り目はこれで開き、2枚が本のように平らに並ぶ
/// - 中央のくさび: ちょうつがいの鏡映
///
/// 部分の順に意味がある: [`flat_motion`] は後の部分ほど `open` の側へ重ねるので、
/// 羽を先・中央を後に並べて中央のくさびを羽の外側に置く。
/// `open` は持ち上げた紙を回す側(手前=Up / 向こう=Down)。
///
/// 持ち上げた紙は**袋ごと**(`pockets`)に、その袋のいちばん外側の層の隣へ置く
/// ([`LayerTurn::Beside`])。重なり全体の外側へまとめて回すと、袋がいくつも
/// 重なったフラップ(カエルの基本形など)で袋の紙が入り混じり、
/// 出来上がった1本の先をつまめなくなる。
fn petal_parts(input: PetalPartsInput<'_>) -> Vec<MotionPart> {
    let PetalPartsInput {
        pockets,
        polygons: polys,
        geometry:
            PetalGeometry {
                tip,
                center_direction: d,
                hinge,
            },
        wings,
        open,
    } = input;
    let seg = |from: DVec2, dir: DVec2| [[from.x, from.y], [from.x + dir.x, from.y + dir.y]];
    let near_side = HalfPlane {
        line: hinge,
        inside_point: [tip.x, tip.y],
    };
    // 領域の内側を示す点は、左右のうち短いほうの縁を基準に取る
    // (長いほうで取るとちょうつがいの向こう側へはみ出すことがある)
    let inner = wings
        .iter()
        .map(|wing| wing.reach)
        .fold(f64::INFINITY, f64::min);
    let mut parts: Vec<MotionPart> = Vec::new();
    let mut middle = vec![near_side.clone()];
    for wing_input in wings {
        let bisector = seg(tip, rotate(d, wing_input.angle * 0.5));
        let outside = tip + rotate(d, wing_input.angle) * (wing_input.reach.min(inner) * 0.5);
        let inside = tip + d * (inner * 0.5);
        let wing_region = vec![
            near_side.clone(),
            HalfPlane {
                line: bisector,
                inside_point: [outside.x, outside.y],
            },
        ];
        middle.push(HalfPlane {
            line: bisector,
            inside_point: [inside.x, inside.y],
        });
        // 隣の層の羽は中心線へ寄せるだけ(もとの層のすぐ上へ入る)。
        // 羽の領域から全部落ちたら部分を作らない(層の指定が空の [`MotionPart`] は
        // 「全ての層」の意味になり、無関係な層まで動いてしまうため)
        let near_layers = layers_in_region(polys, &wing_input.neighbors, &wing_region);
        if !near_layers.is_empty() {
            parts.push(MotionPart {
                layers: near_layers,
                region: wing_region.clone(),
                transform: MotionTransform::Reflect(vec![bisector]),
                turn: LayerTurn::Inside(open),
                reverse_layers: None,
            });
        }
        push_by_pocket(
            &mut parts,
            polys,
            pockets,
            &wing_region,
            MotionTransform::Reflect(vec![bisector, hinge]),
            open,
        );
    }
    push_by_pocket(
        &mut parts,
        polys,
        pockets,
        &middle,
        MotionTransform::Reflect(vec![hinge]),
        open,
    );
    parts
}

/// 持ち上げた紙を袋ごとに1つの部分にし、その袋のいちばん外側の層の隣へ置く。
/// 紙の無い袋(片側にしか紙が無い袋など)は部分を作らない
/// (層の指定が空の [`MotionPart`] は「全ての層」の意味になってしまうため)。
fn push_by_pocket(
    parts: &mut Vec<MotionPart>,
    polys: &HashMap<FaceId, Vec<DVec2>>,
    pockets: &[Vec<FaceId>],
    region: &[HalfPlane],
    transform: MotionTransform,
    open: FoldDirection,
) {
    for pocket in pockets {
        let layers = layers_in_region(polys, pocket, region);
        if layers.is_empty() {
            continue;
        }
        let anchor = match open {
            FoldDirection::Up => *pocket.last().expect("袋には層がある"),
            FoldDirection::Down => pocket[0],
        };
        parts.push(MotionPart {
            layers,
            region: region.to_vec(),
            transform: transform.clone(),
            turn: LayerTurn::Beside {
                anchor,
                direction: open,
            },
            reverse_layers: None,
        });
    }
}

/// 花弁折りのフラップを**袋**ごとに分ける(戻り値は層順=下→上に並べた袋の一覧)。
///
/// 実際の紙の花弁折りは袋を1つずつ折る動きで、持ち上げた紙はその袋の中に留まる。
/// 袋と袋は**中心線に乗った折り目**(袋の口を閉じている背)で背中合わせに
/// つながっているので、その折り目だけを渡らずに層をたどると袋が求まる。
fn petal_pockets(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    flap: &[FaceId],
    l0: DVec2,
    u: DVec2,
) -> Vec<Vec<FaceId>> {
    let pos = vertex_positions(cp);
    let by_edge = faces_by_edge(faces);
    // union-find(袋の代表を親でたどる)
    let mut parent: HashMap<FaceId, FaceId> = flap.iter().map(|&id| (id, id)).collect();
    fn root(parent: &HashMap<FaceId, FaceId>, mut id: FaceId) -> FaceId {
        while parent[&id] != id {
            id = parent[&id];
        }
        id
    }
    for e in &cp.edges {
        let Some(fs) = by_edge.get(&e.id) else {
            continue;
        };
        if fs.len() != 2 || !fs.iter().all(|id| flap.contains(id)) {
            continue;
        }
        let (Some(&p0), Some(&p1)) = (pos.get(&e.v0), pos.get(&e.v1)) else {
            continue;
        };
        let Some(pl) = state.placements.get(&fs[0]) else {
            continue;
        };
        let on_center = |p: DVec2| u.perp_dot(pl.apply(p) - l0).abs() <= JOIN_EPS;
        if on_center(p0) && on_center(p1) {
            continue;
        }
        let (a, b) = (root(&parent, fs[0]), root(&parent, fs[1]));
        if a != b {
            parent.insert(a, b);
        }
    }
    let mut groups: HashMap<FaceId, Vec<FaceId>> = HashMap::new();
    for &id in flap {
        groups.entry(root(&parent, id)).or_default().push(id);
    }
    // 層順序に無い面(重なりに現れない面)は rank が同じになるので、面IDで
    // タイブレークして袋の並びを決定的にする
    let rank = |id: &FaceId| {
        (
            state
                .order
                .iter()
                .position(|x| x == id)
                .unwrap_or(usize::MAX),
            *id,
        )
    };
    let mut out: Vec<Vec<FaceId>> = groups.into_values().collect();
    for g in &mut out {
        g.sort_by_key(rank);
    }
    out.sort_by_key(|g| rank(&g[0]));
    out
}

/// 点 `c` のまわりの角 `angle` の回転(`c` を通る2直線での鏡映の合成)。
fn rotation_about(c: DVec2, angle: f64) -> Isometry2 {
    let half = rotate(DVec2::X, angle * 0.5);
    Isometry2::reflection(c, c + half).compose(&Isometry2::reflection(c, c + DVec2::X))
}

/// 選んだ層が畳み平面で占める範囲(頂点)の重心。
fn flap_centroid(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    flap: &[FaceId],
) -> Option<DVec2> {
    let pos = vertex_positions(cp);
    let (mut sum, mut count) = (DVec2::ZERO, 0usize);
    for f in faces.iter().filter(|f| flap.contains(&f.id)) {
        let pl = state.placements.get(&f.id)?;
        for p in f.vertices.iter().filter_map(|v| pos.get(v)) {
            sum += pl.apply(*p);
            count += 1;
        }
    }
    (count > 0).then(|| sum / count as f64)
}

/// 2点を通る直線(半平面や鏡映へ渡す形)。
fn line_of(p: DVec2, q: DVec2) -> [[f64; 2]; 2] {
    [[p.x, p.y], [q.x, q.y]]
}

/// ねじり折りの中央多角形を、指定された頂点の並びから作る。
///
/// 辺の長さも辺の数も仮定しない。Errにするのは幾何的に多角形にならない指定
/// (頂点3つ未満・続く頂点が重なる・中心が頂点と重なる)だけ。中心が外にある・
/// 凹んでいる指定は、断らずに警告して続ける(「止めずに警告」原則)。
fn polygon_vertices(
    pts: &[[f64; 2]],
    center: DVec2,
    name: &str,
    warnings: &mut Vec<String>,
) -> Result<Vec<DVec2>, String> {
    if pts.len() < 3 {
        return Err(format!(
            "{name}の中央多角形には頂点が3つ以上必要です。頂点を足してください"
        ));
    }
    let v: Vec<DVec2> = pts.iter().map(|p| DVec2::from(*p)).collect();
    let n = v.len();
    for k in 0..n {
        if (v[(k + 1) % n] - v[k]).length() <= EPS {
            return Err(format!(
                "{name}の中央多角形の {k} 番目の辺の長さが0です。重なった頂点を外してください"
            ));
        }
        if (v[k] - center).length() <= EPS {
            return Err(format!(
                "{name}の中央多角形の {k} 番目の頂点が中心と同じ位置です。中心をずらしてください"
            ));
        }
    }
    if !in_polygon(&v, center) {
        warnings.push(format!(
            "この{name}では、中心が中央多角形の外にあります。ひだと腕の向きが定まらないことがあります(指定のまま続行します)"
        ));
    } else {
        let turns: Vec<f64> = (0..n)
            .map(|k| (v[(k + 1) % n] - v[k]).perp_dot(v[(k + 2) % n] - v[(k + 1) % n]))
            .collect();
        if turns.iter().any(|t| *t < 0.0) && turns.iter().any(|t| *t > 0.0) {
            warnings.push(format!(
                "この{name}では、中央多角形が凹んでいます。折り上がりが平らにならないことがあります(指定のまま続行します)"
            ));
        }
    }
    Ok(v)
}

/// ねじり折りの中央多角形を、1辺(`a`-`b`)を中心のまわりに回して作る。
fn regular_polygon(
    a: DVec2,
    b: DVec2,
    center: DVec2,
    name: &str,
    warnings: &mut Vec<String>,
) -> Result<Vec<DVec2>, String> {
    let (ra, rb) = (a - center, b - center);
    if ra.length() <= EPS || rb.length() <= EPS {
        return Err(format!(
            "{name}の中央多角形の辺が中心を通っています。中心から離れた辺を指してください"
        ));
    }
    let span = ra.perp_dot(rb).atan2(ra.dot(rb));
    if span.abs() <= ANGLE_EPS {
        return Err(format!(
            "{name}の中央多角形が作れません。辺の両端が中心から見て同じ向きにあります"
        ));
    }
    let n_f = std::f64::consts::TAU / span.abs();
    let n = n_f.round().max(3.0) as usize;
    if (n_f - n as f64).abs() > 1e-6 {
        warnings.push(format!(
            "この{name}では、指定した辺から中央多角形をちょうど{n}角形に丸めました(指定のまま続行します)"
        ));
    }
    let step = span.signum() * std::f64::consts::TAU / n as f64;
    Ok((0..n)
        .map(|k| center + rotate(ra, step * k as f64))
        .collect())
}

/// ねじり折りの動き。中央の回転・辺ごとのひだ・頂点ごとの腕を組み立てる。
///
/// `v` は中央多角形の頂点(順に並べたもの)、`alpha` はねじる角。
/// 辺の長さも辺の数も仮定しない(頂点ごとの外角から折り線の向きを決める)。
fn twist_parts(
    flap: &[FaceId],
    given: &[FaceId],
    center: DVec2,
    v: &[DVec2],
    alpha: f64,
    open: FoldDirection,
) -> Vec<MotionPart> {
    let layers = if given.is_empty() {
        Vec::new()
    } else {
        flap.to_vec()
    };
    let n = v.len();
    let rot = rotation_about(center, alpha);
    let vp: Vec<DVec2> = v.iter().map(|&p| rot.apply(p)).collect();
    let at = |k: usize| k % n;
    // 辺kの直線(回転前)と、ひだkの等長変換(回転後の辺で折り返す)
    let edges: Vec<[[f64; 2]; 2]> = (0..n).map(|k| line_of(v[k], v[at(k + 1)])).collect();
    let pleat: Vec<Isometry2> = (0..n)
        .map(|k| Isometry2::reflection(vp[k], vp[at(k + 1)]).compose(&rot))
        .collect();

    // 頂点jから外へ出る2本の折り線: p_j(ひだ j-1 との境)と q_j(ひだ j との境)。
    // p_j は中心から外へ向かう放射方向にとり、q_j は「腕がひだの両側と折り目で
    // つながる」条件から決まる(頂点に4本が集まり、平らに畳める形になる)。
    //
    // その条件を解くと q_j は「p_j を頂点jの**外角**(辺 j-1 から辺 j への曲がり角)
    // だけ回した向き」になる。正多角形では外角が 2π/n で一定だが、辺の長さが違う
    // 多角形では頂点ごとに変わるので、頂点ごとに測る。
    let p_dir: Vec<DVec2> = (0..n).map(|j| (v[j] - center).normalize()).collect();
    let q_dir: Vec<DVec2> = (0..n)
        .map(|j| {
            let (before, after) = (v[j] - v[(j + n - 1) % n], v[at(j + 1)] - v[j]);
            let ext = before.perp_dot(after).atan2(before.dot(after));
            rotate(p_dir[j], ext)
        })
        .collect();

    let mut parts: Vec<MotionPart> = Vec::with_capacity(2 * n + 1);
    // 重なりは下から「ひだ → 腕 → 中央」。ひだは元の場所に残し、腕と中央を
    // その上へ回す(どちらの側へ回すかは open_to_back で選べる)
    // ひだ: 辺kの外側で、両端の折り線に挟まれた帯
    for k in 0..n {
        let mid = (v[k] + v[at(k + 1)]) * 0.5;
        // 辺の外側のすぐ近く(辺ごとに中心からの距離で目盛りを取る)
        let inside = mid + (mid - center) * 0.02;
        parts.push(MotionPart {
            layers: layers.clone(),
            region: vec![
                HalfPlane {
                    line: edges[k],
                    inside_point: [inside.x, inside.y],
                },
                HalfPlane {
                    line: line_of(v[k], v[k] + q_dir[k]),
                    inside_point: [inside.x, inside.y],
                },
                HalfPlane {
                    line: line_of(v[at(k + 1)], v[at(k + 1)] + p_dir[at(k + 1)]),
                    inside_point: [inside.x, inside.y],
                },
            ],
            transform: MotionTransform::Isometry(pleat[k]),
            turn: LayerTurn::Keep,
            reverse_layers: None,
        });
    }
    // 腕: 頂点jから出る2本の折り線に挟まれた外側の紙
    for j in 0..n {
        let bis = p_dir[j] + q_dir[j];
        let bis = if bis.length() > EPS {
            bis.normalize()
        } else {
            DVec2::new(-p_dir[j].y, p_dir[j].x)
        };
        let inside = v[j] + bis * (v[j] - center).length();
        let prev = &pleat[(j + n - 1) % n];
        let axis0 = prev.apply(v[j]);
        let axis1 = prev.apply(v[j] + p_dir[j]);
        parts.push(MotionPart {
            layers: layers.clone(),
            region: vec![
                HalfPlane {
                    line: line_of(v[j], v[j] + p_dir[j]),
                    inside_point: [inside.x, inside.y],
                },
                HalfPlane {
                    line: line_of(v[j], v[j] + q_dir[j]),
                    inside_point: [inside.x, inside.y],
                },
            ],
            transform: MotionTransform::Isometry(Isometry2::reflection(axis0, axis1).compose(prev)),
            turn: LayerTurn::Outside(open),
            reverse_layers: None,
        });
    }
    // 中央: 中心まわりの回転
    parts.push(MotionPart {
        layers: layers.clone(),
        region: edges
            .iter()
            .map(|&line| HalfPlane {
                line,
                inside_point: [center.x, center.y],
            })
            .collect(),
        transform: MotionTransform::Isometry(rot),
        turn: LayerTurn::Outside(open),
        reverse_layers: None,
    });
    parts
}

/// 折り線の2点。退化(2点が一致)しているときはErr。
fn line_points(line: [[f64; 2]; 2]) -> Result<(DVec2, DVec2), String> {
    let l0 = DVec2::from(line[0]);
    let l1 = DVec2::from(line[1]);
    if (l1 - l0).length() < EPS {
        return Err("折り線の2点が一致しています".to_string());
    }
    Ok((l0, l1))
}

/// ここまでの折りで畳み平面が裏返っているなら、折る向きを反対にする。
///
/// [`fold_through`] は結果を「根面(最小面ID)が恒等」の座標系へそろえ直すため、
/// 動いた側に根面があると畳み平面ごと裏返る(層順序も上下が入れ替わる)。
/// 技法の2回目以降の折りは1回目と同じ紙の側へ重ねたいので、裏返っている場合は
/// Up/Downを入れ替えて指定する(これをしないと、どの面が最小IDだったかという
/// 内部の事情で技法の結果が変わってしまう)。
fn turn_direction(direction: FoldDirection, turned: bool) -> FoldDirection {
    if !turned {
        return direction;
    }
    match direction {
        FoldDirection::Up => FoldDirection::Down,
        FoldDirection::Down => FoldDirection::Up,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 花弁折りの部分は、層の指定が空のまま作られない。
    ///
    /// [`crate::flat_motion`] は層の指定が空の [`MotionPart`] を「全ての面」と読む。
    /// 隣の層が羽の領域から全部落ちたときに空のまま渡すと、フラップと関係のない
    /// 層まで動いてしまう。
    #[test]
    fn petal_parts_never_leaves_the_layers_empty() {
        let (tip, d) = (DVec2::ZERO, DVec2::X);
        let hinge = [[1.0, -1.0], [1.0, 1.0]];
        let tri = |a: DVec2, b: DVec2, c: DVec2| vec![a, b, c];
        let mut polys: HashMap<FaceId, Vec<DVec2>> = HashMap::new();
        // フラップの層(先端を含む三角形)
        polys.insert(
            1,
            tri(DVec2::ZERO, DVec2::new(1.0, -1.0), DVec2::new(1.0, 1.0)),
        );
        // 隣の層は羽の領域から遠く離れていて、どちらの羽にも掛からない
        polys.insert(
            7,
            tri(
                DVec2::new(5.0, 5.0),
                DVec2::new(6.0, 5.0),
                DVec2::new(6.0, 6.0),
            ),
        );
        let quarter = std::f64::consts::FRAC_PI_4;
        let wings = [
            PetalWing {
                angle: quarter,
                reach: 1.0,
                neighbors: vec![7],
            },
            PetalWing {
                angle: -quarter,
                reach: 1.0,
                neighbors: vec![7],
            },
        ];

        let parts = petal_parts(PetalPartsInput {
            pockets: &[vec![1]],
            polygons: &polys,
            geometry: PetalGeometry {
                tip,
                center_direction: d,
                hinge,
            },
            wings: &wings,
            open: FoldDirection::Up,
        });

        assert!(!parts.is_empty(), "紙のある層は動く");
        for p in &parts {
            assert!(
                !p.layers.is_empty(),
                "層の指定が空の部分は作らない(全ての層が動いてしまう)"
            );
        }
    }
}
