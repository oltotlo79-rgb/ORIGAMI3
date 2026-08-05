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
use ori3_geometry::Isometry2;
use ori3_model::{
    CreasePattern, DriverLine, EPS, EdgeId, EdgeKind, FaceId, FoldStep, TechniqueKind, VertexId,
};

use crate::flat_motion::{
    FlatMotionInput, HalfPlane, LayerTurn, MotionPart, MotionTransform, flat_motion,
};
use crate::flat_state::{FlatState, point_in_face, representative_point};
use crate::fold_through::{
    FoldDirection, FoldThroughInput, FoldThroughResult, TEAR_MARK, angle_of, faces_by_edge,
    fold_through, push_driver_line, vertex_positions,
};

/// 面の配置の一致を見る許容誤差(等長変換の積み重ねで出る誤差より十分大きく取る)。
const JOIN_EPS: f64 = 1e-6;

/// 「向きが同じ」とみなす角度の許容誤差(rad)。つぶし折りの退化ケースの判定に使う。
const ANGLE_EPS: f64 = 1e-9;

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
    let name = if inside { "中割り折り" } else { "かぶせ折り" };
    let (l0, l1) = line_points(input.line)?;
    let u = (l1 - l0).normalize();
    let keep = DVec2::from(input.reference_point);
    if u.perp_dot(keep - l0).abs() <= EPS {
        return Err(format!(
            "{name}の向きを示す点が折り線の上にあります。先端を{}側の点を指してください",
            if inside { "折り込む先の" } else { "かぶせる先の" }
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
    let (up_faces, down_faces): (Vec<FaceId>, Vec<FaceId>) =
        flap.iter().partition(|id| up[id]);

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
        let m = s.fold(line_now, keep_now, Some(&now), turn_direction(direction, turned))?;
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
/// - α=0(基準点が背の延長上)は**退化ケース**: 紙は1mmも動かず、重なり順と
///   背の山谷だけが変わる(2回半分に折った正方形を予備基本形の重なりへ組み替える
///   ときの動き)
///
/// つぶした紙は重なりのいちばん上へ回す(手前へ開く向き)。
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

    let flap = flap_in_layer_order(faces, state, &input.flap, name)?;

    let mut warnings: Vec<String> = Vec::new();
    let (span, pairs) = spine_along(cp, faces, state, &flap, l0, u);
    let span = match span {
        Some(s) => s,
        None => {
            warnings.push(format!(
                "この{name}では、中心線の上に開ける折り目が見つかりません。中心線がフラップの背に重なっているか確かめてください(指定のまま続行します)"
            ));
            flap_span_along(cp, faces, state, &flap, l0, u)
                .ok_or_else(|| format!("{name}の支点が決められません。中心線を引き直してください"))?
        }
    };

    let p = DVec2::from(input.reference_point);
    let (ends0, ends1) = (l0 + u * span.0, l0 + u * span.1);
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

    let (near, far) = split_by_spine(&flap, &pairs, name, &mut warnings);
    let parts = squash_parts(
        &flap,
        &near,
        &far,
        pivot,
        s_dir,
        c_dir,
        alpha,
        (tip - pivot).length(),
    );

    let mut res = flat_motion(
        cp,
        faces,
        state,
        &FlatMotionInput {
            parts,
            kind: TechniqueKind::Squash,
        },
    )?;
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
/// - 中心線に直交する**ちょうつがい1本**: 先端から「縁の長さ」だけ離れた位置を通る。
///   ここで先端側が折り返り、先端が反対の端へ持ち上がる。位置がこう決まるのは、
///   縁を中心線へ寄せたときに縁の端がちょうどこの線へ届くため(この一致が
///   花弁折りが平らに畳める理由)
///
/// 紙の動き(3つの [`MotionPart`]):
///
/// - 両側の羽(斜め線の外側でちょうつがいの手前): 斜め線→ちょうつがいの鏡映2回
///   =回転。縁が中心線へ寄りながら持ち上がる
/// - 中央のくさび(斜め2本の間でちょうつがいの手前): ちょうつがいでの鏡映
/// - ちょうつがいの向こう側の紙は動かない
///
/// 持ち上げた紙は重なりのいちばん上へ回し、中央のくさびを羽の上に置く
/// (羽は先に中心へ折られてから一緒に裏返るので、上下が入れ替わる)。
///
/// 層の数の偶奇やフラップの形は仮定せず、選んだ層すべてが同じように動く。
/// Errにするのは幾何的に決められない入力(退化した中心線・見つからない層・
/// 中心線の向きに広がりの無いフラップ・両側に紙の無いフラップ)だけで、
/// 左右の縁の長さが食い違う・紙が裂けるといった指定は警告して続ける。
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
    let reaches: Vec<f64> = [right, left].iter().filter_map(|s| s.map(|(_, r)| r)).collect();
    if reaches.is_empty() {
        return Err(format!(
            "{name}の先端の両側に紙がありません。中心線と先端の位置を確かめてください"
        ));
    }
    let reach = reaches.iter().sum::<f64>() / reaches.len() as f64;
    if reaches.len() == 2 && (reaches[0] - reaches[1]).abs() > JOIN_EPS {
        warnings.push(format!(
            "この{name}では先端の左右の縁の長さが違います。ちょうつがいの位置を平均で決めるため、このままでは紙が裂けます(指定のまま続行します)"
        ));
    }
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
    let sides: Vec<(f64, Vec<FaceId>)> = [right, left]
        .into_iter()
        .flatten()
        .map(|(ang, _)| {
            let ns = wing_neighbors(cp, faces, state, &flap, tip, d, reach, ang);
            (ang, ns)
        })
        .collect();

    let parts = petal_parts(&flap, tip, d, reach, &sides);
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
    /// 途中の折りで出る「紙が裂けます」の警告は捨てる。技法は複数回の折りで
    /// 1つの形を作るため、1回目だけを見ると必ず層のつながりが切れて見える
    /// (最終形での裂けは [`Session::tear_warnings`] で改めて調べる)。
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
        self.warnings
            .extend(res.warnings.into_iter().filter(|w| !w.contains(TEAR_MARK)));
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
            .filter(|f| {
                self.origin
                    .get(&f.id)
                    .is_some_and(|o| of.contains(o))
            })
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
                    f.vertices.iter().filter_map(|v| pos.get(v)).any(|&p| {
                        keep_sign * u.perp_dot(pl.apply(p) - l0) < -EPS
                    })
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
        let mut adj: HashMap<FaceId, Vec<FaceId>> = flap.iter().map(|&id| (id, Vec::new())).collect();
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
            if !fs.iter().all(|id| self.flipped.get(id).copied() == Some(true)) {
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
            if !fs.iter().any(|id| self.flipped.get(id).copied() == Some(true)) {
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
            note: String::new(),
        };
        *cp = self.cp;
        Ok(FoldThroughResult {
            state: self.state,
            added_edges: added,
            step,
            warnings: self.warnings,
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

/// 中心線に重なっている折り目(開く背)を探す。
///
/// 戻り値は「背が中心線上で占める範囲(l0からの符号付き距離)」と
/// 「フラップの中で背でつながっている面の組」。範囲はフラップの外の面と
/// つながる背も含める(フラップが1層でも支点を決められるように)。
fn spine_along(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    flap: &[FaceId],
    l0: DVec2,
    u: DVec2,
) -> (Option<(f64, f64)>, Vec<[FaceId; 2]>) {
    let pos = vertex_positions(cp);
    let mut span: Option<(f64, f64)> = None;
    let mut pairs: Vec<[FaceId; 2]> = Vec::new();
    for (eid, fs) in faces_by_edge(faces) {
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
        if fs.iter().all(|id| flap.contains(id)) {
            pairs.push([fs[0], fs[1]]);
        }
    }
    (span, pairs)
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
        for t in f.vertices.iter().filter_map(|v| pos.get(v)).map(|&q| u.dot(pl.apply(q) - l0)) {
            lo = lo.min(t);
            hi = hi.max(t);
        }
    }
    (lo < hi).then_some((lo, hi))
}

/// フラップを背で2色に塗り分ける。戻り値は(手前寄り=下の層を含む側, 奥側)。
///
/// 背でつながった2層は開くと反対側へ分かれるので、つながりの図を2色で塗り分ければ
/// どちらの側へ行くかが決まる(層の数を機械的に半分に割らないのは中割り折りと同じ)。
/// 塗り分けられない(奇数の輪になる)場合は、断らずに警告して続ける。
fn split_by_spine(
    flap: &[FaceId],
    pairs: &[[FaceId; 2]],
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
            for pr in pairs {
                let next = if pr[0] == cur {
                    pr[1]
                } else if pr[1] == cur {
                    pr[0]
                } else {
                    continue;
                };
                match color.get(&next) {
                    Some(&v) => odd |= v == cur_color,
                    None => {
                        color.insert(next, !cur_color);
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

/// つぶし折りの動き(手前側=新しい折り線で折り返す / 奥側=層まるごと回る)。
#[allow(clippy::too_many_arguments)]
fn squash_parts(
    flap: &[FaceId],
    near: &[FaceId],
    far: &[FaceId],
    pivot: DVec2,
    s_dir: DVec2,
    c_dir: DVec2,
    alpha: f64,
    reach: f64,
) -> Vec<MotionPart> {
    // 退化ケース: 背が向きを変えないので紙は動かない(重なり順と山谷だけが変わる)
    if alpha.abs() <= ANGLE_EPS {
        return vec![MotionPart::restack(
            flap.to_vec(),
            LayerTurn::Outside(FoldDirection::Up),
        )];
    }
    let seg = |dir: DVec2| [[pivot.x, pivot.y], [pivot.x + dir.x, pivot.y + dir.y]];
    // 新しい折り線: 背の今の向きと行き先の角の二等分線(ここで折ると背が行き先へ向く)
    let (sn, cs) = (alpha * 0.5).sin_cos();
    let m_dir = DVec2::new(s_dir.x * cs - s_dir.y * sn, s_dir.x * sn + s_dir.y * cs);
    let m_line = seg(m_dir);
    let mut parts: Vec<MotionPart> = Vec::new();
    if !near.is_empty() {
        let inside = pivot + s_dir * reach;
        parts.push(MotionPart {
            layers: near.to_vec(),
            region: vec![HalfPlane {
                line: m_line,
                inside_point: [inside.x, inside.y],
            }],
            transform: MotionTransform::Reflect(vec![m_line]),
            turn: LayerTurn::Outside(FoldDirection::Up),
            reverse_layers: None,
        });
    }
    if !far.is_empty() {
        // 鏡映2回=角αの回転。背が開いて(角0°になって)行き先へ向く
        parts.push(MotionPart {
            layers: far.to_vec(),
            region: Vec::new(),
            transform: MotionTransform::Reflect(vec![m_line, seg(c_dir)]),
            turn: LayerTurn::Outside(FoldDirection::Up),
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

/// 羽の領域で、フラップの層と折り目でつながっている「フラップ外の層」。
///
/// 花弁折りでは、羽の外側の縁(フラップの層と隣の層をつないでいる折り目)が
/// **開く**。相手の層の羽も一緒に中心線へ寄せないと、そこで紙が裂ける。
#[allow(clippy::too_many_arguments)]
fn wing_neighbors(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    flap: &[FaceId],
    tip: DVec2,
    d: DVec2,
    reach: f64,
    ang: f64,
) -> Vec<FaceId> {
    let pos = vertex_positions(cp);
    let k = rotate(d, ang * 0.5);
    let planes: [&dyn Fn(DVec2) -> f64; 2] = [
        &|q: DVec2| reach - d.dot(q - tip),
        &|q: DVec2| ang.signum() * k.perp_dot(q - tip),
    ];
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
/// 部分の順に意味がある: [`flat_motion`] は後の部分ほど上に重ねるので、
/// 羽を先・中央を後に並べて中央のくさびを羽の上に置く。
fn petal_parts(
    flap: &[FaceId],
    tip: DVec2,
    d: DVec2,
    reach: f64,
    sides: &[(f64, Vec<FaceId>)],
) -> Vec<MotionPart> {
    let seg = |from: DVec2, dir: DVec2| [[from.x, from.y], [from.x + dir.x, from.y + dir.y]];
    let hinge_at = tip + d * reach;
    let hinge = seg(hinge_at, DVec2::new(-d.y, d.x));
    let near_side = HalfPlane {
        line: hinge,
        inside_point: [tip.x, tip.y],
    };
    let mut parts: Vec<MotionPart> = Vec::new();
    let mut middle = vec![near_side.clone()];
    for (ang, neighbors) in sides {
        let bisector = seg(tip, rotate(d, ang * 0.5));
        let outside = tip + rotate(d, *ang) * reach;
        let inside = tip + d * (reach * 0.5);
        let wing = vec![
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
        if !neighbors.is_empty() {
            // 隣の層の羽は中心線へ寄せるだけ(もとの層のすぐ上へ入る)
            parts.push(MotionPart {
                layers: neighbors.clone(),
                region: wing.clone(),
                transform: MotionTransform::Reflect(vec![bisector]),
                turn: LayerTurn::Inside(FoldDirection::Up),
                reverse_layers: None,
            });
        }
        parts.push(MotionPart {
            layers: flap.to_vec(),
            region: wing,
            transform: MotionTransform::Reflect(vec![bisector, hinge]),
            turn: LayerTurn::Outside(FoldDirection::Up),
            reverse_layers: None,
        });
    }
    parts.push(MotionPart {
        layers: flap.to_vec(),
        region: middle,
        transform: MotionTransform::Reflect(vec![hinge]),
        turn: LayerTurn::Outside(FoldDirection::Up),
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
